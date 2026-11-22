//! Maps the game library with Relink and binds its imports to our bionic shims.

use std::path::Path;

use elf_loader::image::{SyntheticModule, SyntheticSymbol};
use elf_loader::{AddressSpaceId, Domain, Loader, Relocator};
use stable_eyre::eyre::{Result, WrapErr, eyre};

use crate::bionic;

/// Loads, relocates and initializes the library, then resolves each name with `resolve`.
///
/// The library stays mapped for the rest of the process.
#[tracing::instrument(skip_all, fields(path = %path.display()))]
pub(crate) fn load<T>(
    path: &Path,
    resolve: impl FnOnce(&dyn Fn(&str) -> Result<usize>) -> Result<T>,
) -> Result<T> {
    let domain = Domain::native(AddressSpaceId::PROCESS);
    let host = SyntheticModule::new(
        &domain,
        "bionic",
        bionic::imports()
            .into_iter()
            .map(|(name, addr)| SyntheticSymbol::function(name, addr)),
    );

    let raw = Loader::new()
        .run(domain)
        .load_dylib(path)
        .wrap_err("map game library")?;
    let library = Relocator::new()
        .run(raw)
        .with_modules([host])
        .wrap_err("scope host symbols")?
        .relocate()
        .wrap_err("relocate game library")?;
    // The image must outlive exit: the game registered its own atexit handlers inside it.
    let library = std::mem::ManuallyDrop::new(library);
    tracing::debug!("relocated and initialized");

    let lookup = |name: &str| -> Result<usize> {
        // SAFETY: only the address is read; callers cast it to the matching signature.
        let symbol =
            unsafe { library.get::<()>(name) }.wrap_err_with(|| format!("look up {name}"))?;
        symbol
            .map(|s| s.into_raw() as usize)
            .ok_or_else(|| eyre!("library does not export {name}"))
    };
    resolve(&lookup)
}
