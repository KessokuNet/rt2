//! Main-thread FuseAudio PCM pump; SDL consumes queued copies without calling into fake JNI.

use std::cell::RefCell;
use std::ops::RangeInclusive;
use std::ptr;

use jni_sys::{JNIEnv, jclass, jint, jobject};
use sdl3::audio::{AudioDevice, AudioFormat, AudioSpec, AudioStreamOwner};
use sdl3::{AudioSubsystem, Sdl};
use stable_eyre::eyre::{Result, bail, eyre};

use crate::jni::{self, object::JObject};

/// Signature of `Java_com_polarbit_fuse_Jni_AudioMix`; the final argument is frames, not bytes.
pub(crate) type AudioMix =
    unsafe extern "C" fn(env: *mut JNIEnv, class: jclass, buffer: jobject, frames: jint);

const RATE_RANGE: RangeInclusive<i32> = 8_000..=192_000;
const BLOCKS_PER_SECOND: usize = 50;
const QUEUE_BLOCKS: usize = 3;
const FULL_VOLUME: i32 = 100;
const JAVA_ERROR: i32 = -1;

thread_local! {
    static AUDIO: RefCell<Audio> = RefCell::new(Audio::default());
}

struct Audio {
    output: Option<Output>,
    subsystem: Option<AudioSubsystem>,
    sdl: Option<Sdl>,
    mixer: Option<AudioMix>,
    volume: i32,
}

impl Default for Audio {
    fn default() -> Self {
        Self {
            output: None,
            subsystem: None,
            sdl: None,
            mixer: None,
            volume: FULL_VOLUME,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Format {
    rate: i32,
    channels: i32,
    bits: i32,
    frames: usize,
    block_bytes: usize,
}

impl Format {
    fn new(rate: i32, channels: i32, bits: i32, mixbufsize: i32) -> Result<Self> {
        if !RATE_RANGE.contains(&rate) || !matches!(channels, 1 | 2) || !matches!(bits, 8 | 16) {
            bail!("unsupported PCM format: {rate} Hz, {channels} channels, {bits} bits");
        }
        let bytes_per_frame = (channels * (bits / 8)) as usize;
        let capacity = usize::try_from(mixbufsize).map_err(|_| eyre!("negative mixer capacity"))?;
        let desired_frames = (rate as usize).div_ceil(BLOCKS_PER_SECOND);
        let frames = desired_frames.min(capacity / bytes_per_frame);
        if frames == 0 {
            bail!("mixer capacity must hold at least one PCM frame");
        }
        Ok(Self {
            rate,
            channels,
            bits,
            frames,
            block_bytes: frames * bytes_per_frame,
        })
    }

    fn spec(self) -> AudioSpec {
        AudioSpec::new(
            Some(self.rate),
            Some(self.channels),
            Some(if self.bits == 8 {
                AudioFormat::U8
            } else {
                AudioFormat::S16LE
            }),
        )
    }
}

/// Both allocations stay fixed until the mixer stops using the borrowed handle.
struct MixBuffer {
    object: Box<JObject>,
    storage: Box<[u64]>,
    len: usize,
}

impl MixBuffer {
    fn new(len: usize) -> Self {
        let mut storage = vec![0; len.div_ceil(size_of::<u64>())].into_boxed_slice();
        let object = Box::new(JObject::Buffer {
            addr: storage.as_mut_ptr().cast(),
            capacity: len as i64,
        });
        Self {
            object,
            storage,
            len,
        }
    }

    fn handle(&mut self) -> jobject {
        ptr::from_mut(self.object.as_mut()).cast()
    }

    fn bytes(&mut self) -> &mut [u8] {
        // SAFETY: storage is aligned and allocated for at least len bytes, with exclusive access.
        unsafe { std::slice::from_raw_parts_mut(self.storage.as_mut_ptr().cast(), self.len) }
    }
}

struct Output {
    stream: AudioStreamOwner,
    _device: AudioDevice,
    mix: MixBuffer,
    format: Format,
    playing: bool,
    resumed: bool,
}

impl Output {
    fn open(subsystem: &AudioSubsystem, format: Format) -> Result<Self> {
        let spec = format.spec();
        let device = subsystem.open_playback_device(&spec)?;
        if !device.pause() {
            return Err(sdl3::get_error().into());
        }
        let stream = subsystem.new_playback_stream(&spec, None)?;
        device.bind_stream(&stream)?;
        // Own stream before device so destruction unbinds before closing the logical device.
        Ok(Self {
            stream,
            _device: device,
            mix: MixBuffer::new(format.block_bytes),
            format,
            playing: false,
            resumed: false,
        })
    }

    fn stop(&mut self) -> Result<()> {
        self.playing = false;
        self.resumed = false;
        self.stream.pause()?;
        self.stream.clear()?;
        Ok(())
    }

    fn pump(&mut self, mixer: AudioMix, volume: i32) -> Result<()> {
        if !self.playing {
            return Ok(());
        }
        let queued = usize::try_from(self.stream.queued_bytes()?)?;
        let blocks = blocks_needed(queued, self.format.block_bytes);
        tracing::trace!(queued, blocks, "refilling PCM queue");
        for _ in 0..blocks {
            // Silence protects against a mixer that writes less than the requested block.
            self.mix
                .bytes()
                .fill(if self.format.bits == 8 { 128 } else { 0 });
            let buffer = self.mix.handle();
            // SAFETY: the parent keeps the export loaded; this live direct buffer holds frames of PCM.
            unsafe {
                mixer(
                    jni::env(),
                    ptr::null_mut(),
                    buffer,
                    self.format.frames as jint,
                )
            };
            apply_volume(self.mix.bytes(), self.format.bits, volume);
            self.stream.put_data(self.mix.bytes())?;
        }
        if !self.resumed {
            self.stream.resume()?;
            self.resumed = true;
        }
        Ok(())
    }
}

/// Fills towards three source-format blocks, with at most three mixer calls per update.
fn blocks_needed(queued: usize, block_bytes: usize) -> usize {
    let target = block_bytes * QUEUE_BLOCKS;
    (target.saturating_sub(queued) / block_bytes).min(QUEUE_BLOCKS)
}

fn apply_volume(bytes: &mut [u8], bits: i32, volume: i32) {
    if volume == FULL_VOLUME {
        return;
    }
    if bits == 8 {
        for sample in bytes {
            *sample = (128 + (i32::from(*sample) - 128) * volume / FULL_VOLUME) as u8;
        }
    } else {
        for sample in bytes.as_chunks_mut::<2>().0 {
            let value = i32::from(i16::from_le_bytes([sample[0], sample[1]]));
            sample.copy_from_slice(&((value * volume / FULL_VOLUME) as i16).to_le_bytes());
        }
    }
}

/// Initializes SDL audio on the main thread before the game's FuseOnInit/OnCreate calls.
#[tracing::instrument]
pub(crate) fn init() -> Result<()> {
    AUDIO.with_borrow_mut(|audio| {
        if audio.subsystem.is_some() {
            return Ok(());
        }
        let sdl = sdl3::init()?;
        let subsystem = sdl.audio()?;
        tracing::info!(
            driver = subsystem.current_audio_driver(),
            "SDL audio initialized"
        );
        audio.subsystem = Some(subsystem);
        audio.sdl = Some(sdl);
        Ok(())
    })
}

/// The mixer must remain loaded and use the borrowed buffer only during this main-thread call.
pub(crate) unsafe fn set_mixer(mixer: AudioMix) {
    AUDIO.with_borrow_mut(|audio| {
        audio.output = None;
        audio.mixer = Some(mixer);
    });
}

/// Java AudioCreate returns zero on success, including an already-created compatible output.
#[tracing::instrument]
pub(crate) fn create(rate: i32, channels: i32, bits: i32, mixbufsize: i32) -> i32 {
    let result: Result<()> = AUDIO.with_borrow_mut(|audio| {
        let format = Format::new(rate, channels, bits, mixbufsize)?;
        if audio.mixer.is_none() {
            bail!("AudioMix export has not been registered");
        }
        let subsystem = audio
            .subsystem
            .as_ref()
            .ok_or_else(|| eyre!("audio::init was not called"))?;
        if audio
            .output
            .as_ref()
            .is_some_and(|output| output.format == format)
        {
            return Ok(());
        }
        // Open first so a failed format change does not destroy a working output.
        audio.output = Some(Output::open(subsystem, format)?);
        tracing::info!(?format, "PCM output created");
        Ok(())
    });
    match result {
        Ok(()) => 0,
        Err(error) => {
            tracing::error!(%error, "AudioCreate failed");
            JAVA_ERROR
        }
    }
}

/// Java AudioStart returns one on a new start, zero if already started, and minus one on failure.
#[tracing::instrument]
pub(crate) fn start() -> i32 {
    AUDIO.with_borrow_mut(|audio| {
        let Some(output) = audio.output.as_mut() else {
            tracing::error!("AudioStart called without an output");
            return JAVA_ERROR;
        };
        if output.playing {
            return 0;
        }
        // Defer mixing until update, after the engine has returned from its initialization call.
        output.playing = true;
        1
    })
}

/// Pauses the device and discards stale PCM so a later start cannot replay it.
#[tracing::instrument]
pub(crate) fn stop() {
    AUDIO.with_borrow_mut(|audio| {
        if let Some(output) = audio.output.as_mut()
            && let Err(error) = output.stop()
        {
            tracing::error!(%error, "AudioStop failed; closing the device");
            audio.output = None;
        }
    });
}

/// FuseAudio.AudioGetVolume answers one, not the current percentage.
pub(crate) const fn get_volume() -> i32 {
    1
}

/// Android's FuseAudio uses left for both channels; right is intentionally ignored here too.
#[tracing::instrument]
pub(crate) fn set_volume(left: i32, right: i32) {
    AUDIO.with_borrow_mut(|audio| audio.volume = left.clamp(0, FULL_VOLUME));
}

/// Call once per main-thread game frame; errors close the device instead of leaving stale audio playing.
pub(crate) fn update() -> Result<()> {
    AUDIO.with_borrow_mut(|audio| {
        let Some(output) = audio.output.as_mut() else {
            return Ok(());
        };
        let mixer = audio
            .mixer
            .ok_or_else(|| eyre!("AudioMix export is missing"))?;
        let result = output.pump(mixer, audio.volume);
        if result.is_err() {
            audio.output = None;
        }
        result
    })
}

/// Call on the main thread before unloading the game and before tearing down SDL video.
#[tracing::instrument]
pub(crate) fn shutdown() {
    AUDIO.with_borrow_mut(|audio| *audio = Audio::default());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_blocks_respect_frame_size_and_engine_capacity() {
        for channels in [1, 2] {
            for bits in [8, 16] {
                let bpf = (channels * bits / 8) as usize;
                let format = Format::new(22_050, channels, bits, 16_384).unwrap();
                assert_eq!(format.frames, 441);
                assert_eq!(format.block_bytes, 441 * bpf);
                let high_rate = Format::new(192_000, channels, bits, 16_384).unwrap();
                assert_eq!(high_rate.frames, 3840);
                assert_eq!(high_rate.block_bytes, 3840 * bpf);
                let small =
                    Format::new(48_000, channels, bits, (7 * bpf + bpf - 1) as i32).unwrap();
                assert_eq!(small.frames, 7);
                assert_eq!(small.block_bytes, 7 * bpf);
            }
        }
        for args in [
            (0, 2, 16, 4096),
            (44_100, 3, 16, 4096),
            (44_100, 2, 24, 4096),
            (44_100, 2, 16, -1),
            (44_100, 2, 16, 0),
            (44_100, 2, 16, 3),
        ] {
            assert!(Format::new(args.0, args.1, args.2, args.3).is_err());
        }
    }

    #[test]
    fn queue_refill_is_bounded_without_partial_block_overshoot() {
        let block = 2048;
        assert_eq!(blocks_needed(0, block), 3);
        assert_eq!(blocks_needed(block, block), 2);
        assert_eq!(blocks_needed(block + 1, block), 1);
        assert_eq!(blocks_needed(block * 3 - 1, block), 0);
        assert_eq!(blocks_needed(block * 3, block), 0);
        assert_eq!(blocks_needed(usize::MAX, block), 0);
        for queued in 0..=block * 4 {
            let added = blocks_needed(queued, block) * block;
            assert!(added <= block * QUEUE_BLOCKS);
            assert!(queued + added <= (block * QUEUE_BLOCKS).max(queued));
        }
    }

    #[test]
    fn unsigned_and_signed_pcm_volume_preserves_silence_and_sign() {
        let mut u8_pcm = [0, 128, 255];
        apply_volume(&mut u8_pcm, 8, 50);
        assert_eq!(u8_pcm, [64, 128, 191]);
        apply_volume(&mut u8_pcm, 8, 0);
        assert_eq!(u8_pcm, [128; 3]);
        let mut s16_pcm: Vec<u8> = [i16::MIN, -2, 0, 2, i16::MAX]
            .into_iter()
            .flat_map(i16::to_le_bytes)
            .collect();
        apply_volume(&mut s16_pcm, 16, 50);
        let samples: Vec<i16> = s16_pcm
            .as_chunks::<2>()
            .0
            .iter()
            .map(|s| i16::from_le_bytes([s[0], s[1]]))
            .collect();
        assert_eq!(samples, [-16_384, -1, 0, 1, 16_383]);
        apply_volume(&mut s16_pcm, 16, 0);
        assert!(s16_pcm.iter().all(|&byte| byte == 0));
    }
}
