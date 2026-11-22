#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

typedef union {
    uint8_t z;
    int8_t b;
    uint16_t c;
    int16_t s;
    int32_t i;
    int64_t j;
    float f;
    double d;
    void *l;
} jvalue;

enum { JNI_SLOTS = 240, MAX_ARGS = 16 };

extern const char *rt2_jni_method_sig(const void *mid);
extern void rt2_jni_invoke(void *obj, const void *mid, const jvalue *args, jvalue *out);
extern void rt2_jni_trap(int slot);
extern void rt2_android_log(int prio, const char *tag, const char *msg);

int rt2_android_log_print(int prio, const char *tag, const char *fmt, ...)
{
    char msg[1024];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(msg, sizeof(msg), fmt, ap);
    va_end(ap);
    rt2_android_log(prio, tag, msg);
    return 0;
}

extern void *rt2_jni_new_instance(void *clazz);

/* NewObject, NewObjectV and NewObjectA only need the class: our objects carry no fields. */
void *rt2_jni_new_object(void *env, void *clazz, const void *mid, ...)
{
    (void)env;
    (void)mid;
    return rt2_jni_new_instance(clazz);
}

void *rt2_jni_new_object_v(void *env, void *clazz, const void *mid, va_list ap)
{
    (void)env;
    (void)mid;
    (void)ap;
    return rt2_jni_new_instance(clazz);
}

void *rt2_jni_new_object_a(void *env, void *clazz, const void *mid, const jvalue *args)
{
    (void)env;
    (void)mid;
    (void)args;
    return rt2_jni_new_instance(clazz);
}

static const char *skip_type(const char *p)
{
    while (*p == '[')
        p++;
    if (*p == 'L') {
        p = strchr(p, ';');
        return p ? p + 1 : "";
    }
    return *p ? p + 1 : p;
}

static void decode_va(const char *sig, va_list src, jvalue *out)
{
    va_list ap;
    va_copy(ap, src);
    const char *p = strchr(sig, '(');
    p = p ? p + 1 : sig;
    for (int n = 0; *p && *p != ')' && n < MAX_ARGS; n++) {
        switch (*p) {
        case 'Z':
        case 'B':
        case 'C':
        case 'S':
        case 'I': out[n].i = va_arg(ap, int32_t); break;
        case 'J': out[n].j = va_arg(ap, int64_t); break;
        case 'F': out[n].f = (float)va_arg(ap, double); break;
        case 'D': out[n].d = va_arg(ap, double); break;
        default: out[n].l = va_arg(ap, void *); break;
        }
        p = skip_type(p);
    }
    va_end(ap);
}

static jvalue call_v(void *obj, const void *mid, va_list ap)
{
    jvalue args[MAX_ARGS] = {0};
    jvalue out = {0};
    decode_va(rt2_jni_method_sig(mid), ap, args);
    rt2_jni_invoke(obj, mid, args, &out);
    return out;
}

static jvalue call_a(void *obj, const void *mid, const jvalue *args)
{
    jvalue out = {0};
    rt2_jni_invoke(obj, mid, args, &out);
    return out;
}

#define CALL_FAMILY(Name, ctype, field) \
    static ctype Call##Name##MethodV(void *env, void *obj, const void *mid, va_list ap) \
    { \
        (void)env; \
        return (ctype)call_v(obj, mid, ap).field; \
    } \
    static ctype Call##Name##Method(void *env, void *obj, const void *mid, ...) \
    { \
        (void)env; \
        va_list ap; \
        va_start(ap, mid); \
        ctype r = (ctype)call_v(obj, mid, ap).field; \
        va_end(ap); \
        return r; \
    } \
    static ctype Call##Name##MethodA(void *env, void *obj, const void *mid, const jvalue *args) \
    { \
        (void)env; \
        return (ctype)call_a(obj, mid, args).field; \
    }

CALL_FAMILY(Object, void *, l)
CALL_FAMILY(Boolean, uint8_t, z)
CALL_FAMILY(Byte, int8_t, b)
CALL_FAMILY(Char, uint16_t, c)
CALL_FAMILY(Short, int16_t, s)
CALL_FAMILY(Int, int32_t, i)
CALL_FAMILY(Long, int64_t, j)
CALL_FAMILY(Float, float, f)
CALL_FAMILY(Double, double, d)

static void CallVoidMethodV(void *env, void *obj, const void *mid, va_list ap)
{
    (void)env;
    call_v(obj, mid, ap);
}

static void CallVoidMethod(void *env, void *obj, const void *mid, ...)
{
    (void)env;
    va_list ap;
    va_start(ap, mid);
    call_v(obj, mid, ap);
    va_end(ap);
}

static void CallVoidMethodA(void *env, void *obj, const void *mid, const jvalue *args)
{
    (void)env;
    call_a(obj, mid, args);
}

#define FAMILY(Name) {(void *)Call##Name##Method, (void *)Call##Name##MethodV, (void *)Call##Name##MethodA}

/* Plain, V and A entry points for each JNI return type, in JNINativeInterface order. */
void *const rt2_jni_call_families[10][3] = {
    FAMILY(Object), FAMILY(Boolean), FAMILY(Byte),   FAMILY(Char),   FAMILY(Short),
    FAMILY(Int),    FAMILY(Long),    FAMILY(Float),  FAMILY(Double), FAMILY(Void),
};

#define S10(X, p) X(p##0) X(p##1) X(p##2) X(p##3) X(p##4) X(p##5) X(p##6) X(p##7) X(p##8) X(p##9)
#define S100(X, p) \
    S10(X, p##0) S10(X, p##1) S10(X, p##2) S10(X, p##3) S10(X, p##4) S10(X, p##5) S10(X, p##6) S10(X, p##7) \
        S10(X, p##8) S10(X, p##9)
#define ALL_SLOTS(X) \
    S10(X, ) S10(X, 1) S10(X, 2) S10(X, 3) S10(X, 4) S10(X, 5) S10(X, 6) S10(X, 7) S10(X, 8) S10(X, 9) \
        S100(X, 1) S10(X, 20) S10(X, 21) S10(X, 22) S10(X, 23)
#define DEF_TRAP(n) \
    static void trap_##n(void) { rt2_jni_trap(n); }
#define REF_TRAP(n) [n] = (void *)trap_##n,

ALL_SLOTS(DEF_TRAP)

/* One aborting stub per JNIEnv slot, so an unimplemented call reports its index. */
void *const rt2_jni_traps[JNI_SLOTS] = {ALL_SLOTS(REF_TRAP)};
