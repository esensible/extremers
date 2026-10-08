#include "solid_host.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include "quickjs.h"

#define SW_MAX_DEPTH 64

struct sw_engine {
    JSRuntime *rt;
    JSContext *ctx;
    sw_host_callbacks cb;
    uint64_t deadline_ns;   // 0 = none
    int interrupted;
    int depth;              // nesting of sw_engine_* calls
    int recording;
    char host_err[512];
    char last_stack[4096];
};

/// Reports JSON.stringify([op, ...args]) to the record callback.
static void record(JSContext *ctx, const char *op, int argc, JSValueConst *argv) {
    sw_engine *e = (sw_engine *)JS_GetContextOpaque(ctx);
    if (!e->recording || !e->cb.record) return;
    JSValue arr = JS_NewArray(ctx);
    if (JS_IsException(arr)) { JS_FreeValue(ctx, JS_GetException(ctx)); return; }
    JS_SetPropertyUint32(ctx, arr, 0, JS_NewString(ctx, op));
    for (int i = 0; i < argc; i++)
        JS_SetPropertyUint32(ctx, arr, (uint32_t)i + 1, JS_DupValue(ctx, argv[i]));
    JSValue s = JS_JSONStringify(ctx, arr, JS_UNDEFINED, JS_UNDEFINED);
    JS_FreeValue(ctx, arr);
    if (JS_IsException(s)) { JS_FreeValue(ctx, JS_GetException(ctx)); return; }
    size_t n = 0;
    const char *str = JS_ToCStringLen(ctx, &n, s);
    if (str) {
        e->cb.record(e->cb.user, str, n);
        JS_FreeCString(ctx, str);
    }
    JS_FreeValue(ctx, s);
}

/// argv[i], or undefined when absent.
static JSValueConst arg_or_undefined(int argc, JSValueConst *argv, int i) {
    return i < argc ? argv[i] : JS_UNDEFINED;
}

static uint64_t now_ns(void) {
#ifdef __APPLE__
    return clock_gettime_nsec_np(CLOCK_UPTIME_RAW);
#else
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ull + (uint64_t)ts.tv_nsec;
#endif
}

static sw_engine *engine_of(JSContext *ctx) {
    return (sw_engine *)JS_GetContextOpaque(ctx);
}

// ---------------------------------------------------------------------------
// Growable byte buffer + value codec

typedef struct buf {
    uint8_t *p;
    size_t len, cap;
    int oom;
} buf;

static void b_reserve(buf *b, size_t n) {
    if (b->oom || b->len + n <= b->cap) return;
    size_t cap = b->cap ? b->cap : 64;
    while (cap < b->len + n) cap *= 2;
    uint8_t *p = realloc(b->p, cap);
    if (!p) { b->oom = 1; return; }
    b->p = p;
    b->cap = cap;
}

static void b_put(buf *b, const void *src, size_t n) {
    b_reserve(b, n);
    if (b->oom) return;
    memcpy(b->p + b->len, src, n);
    b->len += n;
}

static void b_u8(buf *b, uint8_t v) { b_put(b, &v, 1); }

static void b_u32(buf *b, uint32_t v) {
    uint8_t t[4] = { v & 0xff, (v >> 8) & 0xff, (v >> 16) & 0xff, (v >> 24) & 0xff };
    b_put(b, t, 4);
}

static void b_f64(buf *b, double d) {
    uint64_t u;
    memcpy(&u, &d, 8);
    uint8_t t[8];
    for (int i = 0; i < 8; i++) t[i] = (u >> (8 * i)) & 0xff;
    b_put(b, t, 8);
}

static void b_str(buf *b, const char *s, size_t n) {
    b_u32(b, (uint32_t)n);
    b_put(b, s, n);
}

/// Encodes `v` into `b`. Returns -1 with a pending JS exception on failure.
static int encode(JSContext *ctx, buf *b, JSValueConst v, int depth) {
    if (depth > SW_MAX_DEPTH) {
        JS_ThrowRangeError(ctx, "value nested too deeply");
        return -1;
    }
    if (JS_IsUndefined(v)) { b_u8(b, SW_V_UNDEFINED); return 0; }
    if (JS_IsNull(v)) { b_u8(b, SW_V_NULL); return 0; }
    if (JS_IsBool(v)) { b_u8(b, JS_ToBool(ctx, v) ? SW_V_TRUE : SW_V_FALSE); return 0; }
    if (JS_IsNumber(v)) {
        double d = 0;
        if (JS_ToFloat64(ctx, &d, v)) return -1;
        b_u8(b, SW_V_NUMBER);
        b_f64(b, d);
        return 0;
    }
    if (JS_IsString(v)) {
        size_t n = 0;
        const char *s = JS_ToCStringLen(ctx, &n, v);
        if (!s) return -1;
        b_u8(b, SW_V_STRING);
        b_str(b, s, n);
        JS_FreeCString(ctx, s);
        return 0;
    }
    if (JS_IsFunction(ctx, v)) { b_u8(b, SW_V_FUNCTION); return 0; }
    if (!JS_IsObject(v)) {
        // symbol, bigint: pass their string form
        size_t n = 0;
        const char *s = JS_ToCStringLen(ctx, &n, v);
        if (!s) return -1;
        b_u8(b, SW_V_STRING);
        b_str(b, s, n);
        JS_FreeCString(ctx, s);
        return 0;
    }
    if (JS_GetTypedArrayType(v) == JS_TYPED_ARRAY_UINT8 ||
        JS_GetTypedArrayType(v) == JS_TYPED_ARRAY_UINT8C) {
        size_t n = 0;
        uint8_t *p = JS_GetUint8Array(ctx, &n, v);
        if (!p && n) return -1;
        b_u8(b, SW_V_BYTES);
        b_u32(b, (uint32_t)n);
        if (n) b_put(b, p, n);
        return 0;
    }
    if (JS_IsArrayBuffer(v)) {
        size_t n = 0;
        uint8_t *p = JS_GetArrayBuffer(ctx, &n, v);
        if (!p && n) return -1;
        b_u8(b, SW_V_BYTES);
        b_u32(b, (uint32_t)n);
        if (n) b_put(b, p, n);
        return 0;
    }
    if (JS_IsArray(v) || JS_GetTypedArrayType(v) >= 0) {
        int64_t n = 0;
        if (JS_GetLength(ctx, v, &n)) return -1;
        b_u8(b, SW_V_ARRAY);
        b_u32(b, (uint32_t)n);
        for (int64_t i = 0; i < n; i++) {
            JSValue item = JS_GetPropertyUint32(ctx, v, (uint32_t)i);
            if (JS_IsException(item)) return -1;
            int rc = encode(ctx, b, item, depth + 1);
            JS_FreeValue(ctx, item);
            if (rc) return -1;
        }
        return 0;
    }
    JSPropertyEnum *tab = NULL;
    uint32_t count = 0;
    if (JS_GetOwnPropertyNames(ctx, &tab, &count, v, JS_GPN_STRING_MASK | JS_GPN_ENUM_ONLY))
        return -1;
    b_u8(b, SW_V_OBJECT);
    b_u32(b, count);
    int rc = 0;
    for (uint32_t i = 0; i < count && rc == 0; i++) {
        size_t n = 0;
        const char *k = JS_AtomToCStringLen(ctx, &n, tab[i].atom);
        if (!k) { rc = -1; break; }
        b_str(b, k, n);
        JS_FreeCString(ctx, k);
        JSValue item = JS_GetProperty(ctx, v, tab[i].atom);
        if (JS_IsException(item)) { rc = -1; break; }
        rc = encode(ctx, b, item, depth + 1);
        JS_FreeValue(ctx, item);
    }
    JS_FreePropertyEnum(ctx, tab, count);
    return rc;
}

typedef struct reader {
    const uint8_t *p;
    size_t len, pos;
} reader;

static int r_need(reader *r, size_t n) { return r->pos + n <= r->len; }

static uint32_t r_u32(reader *r) {
    const uint8_t *p = r->p + r->pos;
    r->pos += 4;
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) | ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
}

/// Decodes one value. Returns JS_EXCEPTION (with a pending exception) on
/// malformed input.
static JSValue decode(JSContext *ctx, reader *r, int depth) {
    if (depth > SW_MAX_DEPTH) return JS_ThrowRangeError(ctx, "value nested too deeply");
    if (!r_need(r, 1)) return JS_ThrowTypeError(ctx, "truncated host value");
    uint8_t t = r->p[r->pos++];
    switch (t) {
    case SW_V_UNDEFINED: return JS_UNDEFINED;
    case SW_V_NULL: return JS_NULL;
    case SW_V_FALSE: return JS_FALSE;
    case SW_V_TRUE: return JS_TRUE;
    case SW_V_NUMBER: {
        if (!r_need(r, 8)) break;
        uint64_t u = 0;
        for (int i = 0; i < 8; i++) u |= (uint64_t)r->p[r->pos + i] << (8 * i);
        r->pos += 8;
        double d;
        memcpy(&d, &u, 8);
        return JS_NewNumber(ctx, d);
    }
    case SW_V_STRING:
    case SW_V_BYTES:
    case SW_V_JSON: {
        if (!r_need(r, 4)) break;
        uint32_t n = r_u32(r);
        if (!r_need(r, n)) break;
        const char *s = (const char *)r->p + r->pos;
        r->pos += n;
        if (t == SW_V_STRING) return JS_NewStringLen(ctx, s, n);
        if (t == SW_V_BYTES) return JS_NewUint8ArrayCopy(ctx, (const uint8_t *)s, n);
        // JS_ParseJSON needs a NUL-terminated buffer.
        char *tmp = malloc(n + 1);
        if (!tmp) return JS_ThrowOutOfMemory(ctx);
        memcpy(tmp, s, n);
        tmp[n] = 0;
        JSValue v = JS_ParseJSON(ctx, tmp, n, "<state>");
        free(tmp);
        return v;
    }
    case SW_V_ARRAY: {
        if (!r_need(r, 4)) break;
        uint32_t n = r_u32(r);
        JSValue arr = JS_NewArray(ctx);
        if (JS_IsException(arr)) return arr;
        for (uint32_t i = 0; i < n; i++) {
            JSValue item = decode(ctx, r, depth + 1);
            if (JS_IsException(item)) { JS_FreeValue(ctx, arr); return item; }
            if (JS_SetPropertyUint32(ctx, arr, i, item) < 0) { JS_FreeValue(ctx, arr); return JS_EXCEPTION; }
        }
        return arr;
    }
    case SW_V_OBJECT: {
        if (!r_need(r, 4)) break;
        uint32_t n = r_u32(r);
        JSValue obj = JS_NewObject(ctx);
        if (JS_IsException(obj)) return obj;
        for (uint32_t i = 0; i < n; i++) {
            if (!r_need(r, 4)) { JS_FreeValue(ctx, obj); return JS_ThrowTypeError(ctx, "truncated host value"); }
            uint32_t kn = r_u32(r);
            if (!r_need(r, kn)) { JS_FreeValue(ctx, obj); return JS_ThrowTypeError(ctx, "truncated host value"); }
            JSAtom key = JS_NewAtomLen(ctx, (const char *)r->p + r->pos, kn);
            r->pos += kn;
            if (key == JS_ATOM_NULL) { JS_FreeValue(ctx, obj); return JS_EXCEPTION; }
            JSValue item = decode(ctx, r, depth + 1);
            if (JS_IsException(item)) { JS_FreeAtom(ctx, key); JS_FreeValue(ctx, obj); return item; }
            int rc = JS_DefinePropertyValue(ctx, obj, key, item, JS_PROP_C_W_E);
            JS_FreeAtom(ctx, key);
            if (rc < 0) { JS_FreeValue(ctx, obj); return JS_EXCEPTION; }
        }
        return obj;
    }
    default:
        return JS_ThrowTypeError(ctx, "bad host value tag 0x%02x", t);
    }
    return JS_ThrowTypeError(ctx, "truncated host value");
}

// ---------------------------------------------------------------------------
// __host and console

static JSValue host_fail(JSContext *ctx, const char *fn) {
    sw_engine *e = engine_of(ctx);
    JSValue r = JS_ThrowTypeError(ctx, "__host.%s: %s", fn,
                                  e->host_err[0] ? e->host_err : "failed");
    e->host_err[0] = 0;
    return r;
}

static int32_t arg_i32(JSContext *ctx, int argc, JSValueConst *argv, int i, int32_t dflt) {
    int32_t v = dflt;
    if (i < argc && !JS_IsUndefined(argv[i]) && !JS_IsNull(argv[i]))
        JS_ToInt32(ctx, &v, argv[i]);
    return v;
}

static JSValue h_log(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    sw_engine *e = engine_of(ctx);
    // Space-joined string forms, like console.log.
    buf b = {0};
    for (int i = 0; i < argc; i++) {
        size_t n = 0;
        const char *s = JS_ToCStringLen(ctx, &n, argv[i]);
        if (!s) { free(b.p); return JS_EXCEPTION; }
        if (i) b_u8(&b, ' ');
        b_put(&b, s, n);
        JS_FreeCString(ctx, s);
    }
    b_u8(&b, 0);
    if (b.oom) { free(b.p); return JS_ThrowOutOfMemory(ctx); }
    if (e->cb.log) e->cb.log(e->cb.user, (const char *)b.p, b.len - 1);
    free(b.p);
    return JS_UNDEFINED;
}

static JSValue h_create(JSContext *ctx, int argc, JSValueConst *argv, int text) {
    sw_engine *e = engine_of(ctx);
    if (argc < 1) return JS_ThrowTypeError(ctx, text ? "createText(text)" : "createNode(type)");
    size_t n = 0;
    const char *s = JS_ToCStringLen(ctx, &n, argv[0]);
    if (!s) return JS_EXCEPTION;
    int32_t tag = -1;
    if (text) {
        if (e->cb.create_text) tag = e->cb.create_text(e->cb.user, s, n);
    } else {
        if (e->cb.create_node) tag = e->cb.create_node(e->cb.user, s, n);
    }
    JS_FreeCString(ctx, s);
    if (tag <= 0) return host_fail(ctx, text ? "createText" : "createNode");
    JSValue rec_args[2] = { JS_NewInt32(ctx, tag), argv[0] };
    record(ctx, text ? "createText" : "createNode", 2, rec_args);
    return JS_NewInt32(ctx, tag);
}

static JSValue h_create_node(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    return h_create(ctx, argc, argv, 0);
}

static JSValue h_create_text(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    return h_create(ctx, argc, argv, 1);
}

static JSValue h_set_prop(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    sw_engine *e = engine_of(ctx);
    if (argc < 2) return JS_ThrowTypeError(ctx, "setProp(tag, key, value)");
    {
        JSValueConst v = arg_or_undefined(argc, argv, 2);
        JSValueConst rec_args[3] = { argv[0], argv[1], JS_IsUndefined(v) ? JS_NULL : v };
        record(ctx, "setProp", 3, (JSValue *)rec_args);
    }
    int32_t tag = arg_i32(ctx, argc, argv, 0, -1);
    size_t kn = 0;
    const char *key = JS_ToCStringLen(ctx, &kn, argv[1]);
    if (!key) return JS_EXCEPTION;
    buf b = {0};
    if (encode(ctx, &b, argc > 2 ? argv[2] : JS_UNDEFINED, 0)) {
        JS_FreeCString(ctx, key);
        free(b.p);
        return JS_EXCEPTION;
    }
    if (b.oom) { JS_FreeCString(ctx, key); free(b.p); return JS_ThrowOutOfMemory(ctx); }
    int rc = e->cb.set_prop ? e->cb.set_prop(e->cb.user, tag, key, kn, b.p, b.len) : 0;
    JS_FreeCString(ctx, key);
    free(b.p);
    if (rc) return host_fail(ctx, "setProp");
    return JS_UNDEFINED;
}

static void record_fixed(JSContext *ctx, const char *op, int arity, int argc, JSValueConst *argv) {
    JSValueConst a[3];
    for (int i = 0; i < arity && i < 3; i++) a[i] = arg_or_undefined(argc, argv, i);
    record(ctx, op, arity, (JSValue *)a);
}

static JSValue h_insert(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    sw_engine *e = engine_of(ctx);
    record_fixed(ctx, "insert", 3, argc, argv);
    int rc = e->cb.insert ? e->cb.insert(e->cb.user, arg_i32(ctx, argc, argv, 0, -1),
                                         arg_i32(ctx, argc, argv, 1, -1),
                                         arg_i32(ctx, argc, argv, 2, 0)) : 0;
    if (rc) return host_fail(ctx, "insert");
    return JS_UNDEFINED;
}

static JSValue h_remove(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    sw_engine *e = engine_of(ctx);
    record_fixed(ctx, "remove", 2, argc, argv);
    int rc = e->cb.remove ? e->cb.remove(e->cb.user, arg_i32(ctx, argc, argv, 0, -1),
                                         arg_i32(ctx, argc, argv, 1, -1)) : 0;
    if (rc) return host_fail(ctx, "remove");
    return JS_UNDEFINED;
}

static JSValue h_commit(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    sw_engine *e = engine_of(ctx);
    record(ctx, "commit", 0, NULL);
    int rc = e->cb.commit ? e->cb.commit(e->cb.user) : 0;
    if (rc) return host_fail(ctx, "commit");
    return JS_UNDEFINED;
}

static JSValue h_set_timer(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    sw_engine *e = engine_of(ctx);
    record_fixed(ctx, "setTimer", 2, argc, argv);
    double ms = 0;
    if (argc < 2 || JS_ToFloat64(ctx, &ms, argv[1])) return JS_ThrowTypeError(ctx, "setTimer(id, ms)");
    int rc = e->cb.set_timer ? e->cb.set_timer(e->cb.user, arg_i32(ctx, argc, argv, 0, -1), ms) : 0;
    if (rc) return host_fail(ctx, "setTimer");
    return JS_UNDEFINED;
}

static JSValue h_clear_timer(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    sw_engine *e = engine_of(ctx);
    record_fixed(ctx, "clearTimer", 1, argc, argv);
    int rc = e->cb.clear_timer ? e->cb.clear_timer(e->cb.user, arg_i32(ctx, argc, argv, 0, -1)) : 0;
    if (rc) return host_fail(ctx, "clearTimer");
    return JS_UNDEFINED;
}

static JSValue h_send(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    sw_engine *e = engine_of(ctx);
    record_fixed(ctx, "send", 1, argc, argv);
    buf b = {0};
    if (encode(ctx, &b, argc > 0 ? argv[0] : JS_UNDEFINED, 0)) {
        free(b.p);
        return JS_EXCEPTION;
    }
    if (b.oom) { free(b.p); return JS_ThrowOutOfMemory(ctx); }
    int rc = e->cb.send ? e->cb.send(e->cb.user, b.p, b.len) : 0;
    free(b.p);
    if (rc) return host_fail(ctx, "send");
    return JS_UNDEFINED;
}

static JSValue h_release(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    sw_engine *e = engine_of(ctx);
    int rc = e->cb.release ? e->cb.release(e->cb.user, arg_i32(ctx, argc, argv, 0, -1)) : 0;
    if (rc) return host_fail(ctx, "release");
    return JS_UNDEFINED;
}

static const JSCFunctionListEntry host_funcs[] = {
    JS_CFUNC_DEF("setTimer", 2, h_set_timer),
    JS_CFUNC_DEF("clearTimer", 1, h_clear_timer),
    JS_CFUNC_DEF("send", 1, h_send),
    JS_CFUNC_DEF("release", 1, h_release),
    JS_CFUNC_DEF("log", 1, h_log),
    JS_CFUNC_DEF("createNode", 1, h_create_node),
    JS_CFUNC_DEF("createText", 1, h_create_text),
    JS_CFUNC_DEF("setProp", 3, h_set_prop),
    JS_CFUNC_DEF("insert", 3, h_insert),
    JS_CFUNC_DEF("remove", 2, h_remove),
    JS_CFUNC_DEF("commit", 0, h_commit),
};

static const JSCFunctionListEntry console_funcs[] = {
    JS_CFUNC_DEF("log", 1, h_log),
    JS_CFUNC_DEF("info", 1, h_log),
    JS_CFUNC_DEF("warn", 1, h_log),
    JS_CFUNC_DEF("error", 1, h_log),
};

static int interrupt_handler(JSRuntime *rt, void *opaque) {
    sw_engine *e = opaque;
    if (e->deadline_ns && now_ns() > e->deadline_ns) {
        e->interrupted = 1;
        return 1;
    }
    return 0;
}

// ---------------------------------------------------------------------------
// Engine

sw_engine *sw_engine_new(const sw_host_callbacks *cb, const sw_engine_options *opt) {
    sw_engine *e = calloc(1, sizeof(*e));
    if (!e) return NULL;
    e->cb = *cb;
    e->rt = JS_NewRuntime();
    if (!e->rt) { free(e); return NULL; }
    if (opt && opt->memory_limit) JS_SetMemoryLimit(e->rt, opt->memory_limit);
    if (opt && opt->max_stack_size) JS_SetMaxStackSize(e->rt, opt->max_stack_size);
    JS_SetInterruptHandler(e->rt, interrupt_handler, e);
    e->ctx = JS_NewContext(e->rt);
    if (!e->ctx) { JS_FreeRuntime(e->rt); free(e); return NULL; }
    JS_SetContextOpaque(e->ctx, e);

    JSValue global = JS_GetGlobalObject(e->ctx);
    JSValue host = JS_NewObject(e->ctx);
    JS_SetPropertyFunctionList(e->ctx, host, host_funcs,
                               sizeof(host_funcs) / sizeof(host_funcs[0]));
    JS_SetPropertyStr(e->ctx, global, "__host", host);
    JSValue console = JS_NewObject(e->ctx);
    JS_SetPropertyFunctionList(e->ctx, console, console_funcs,
                               sizeof(console_funcs) / sizeof(console_funcs[0]));
    JS_SetPropertyStr(e->ctx, global, "console", console);
    JS_FreeValue(e->ctx, global);
    return e;
}

void sw_engine_free(sw_engine *e) {
    if (!e) return;
    JS_FreeContext(e->ctx);
    JS_FreeRuntime(e->rt);
    free(e);
}

void sw_engine_set_host_error(sw_engine *e, const char *msg) {
    if (!e) return;
    snprintf(e->host_err, sizeof(e->host_err), "%s", msg ? msg : "");
}

void sw_free(void *p) { free(p); }

void sw_engine_set_recording(sw_engine *e, int on) {
    if (e) e->recording = on;
}

const char *sw_engine_last_stack(sw_engine *e) { return e ? e->last_stack : ""; }

char *sw_engine_stringify(sw_engine *e, const uint8_t *value, size_t len) {
    if (!e) return NULL;
    JSContext *ctx = e->ctx;
    reader rd = { value, len, 0 };
    JSValue v = decode(ctx, &rd, 0);
    if (JS_IsException(v)) { JS_FreeValue(ctx, JS_GetException(ctx)); return NULL; }
    JSValue s = JS_JSONStringify(ctx, v, JS_UNDEFINED, JS_UNDEFINED);
    JS_FreeValue(ctx, v);
    if (JS_IsException(s)) { JS_FreeValue(ctx, JS_GetException(ctx)); return NULL; }
    size_t n = 0;
    const char *str = JS_ToCStringLen(ctx, &n, s);
    char *out = NULL;
    if (str) {
        out = malloc(n + 1);
        if (out) { memcpy(out, str, n); out[n] = 0; }
        JS_FreeCString(ctx, str);
    }
    JS_FreeValue(ctx, s);
    return out;
}

static void begin(sw_engine *e, double timeout_ms) {
    if (e->depth++ == 0) {
        e->interrupted = 0;
        e->deadline_ns = timeout_ms > 0 ? now_ns() + (uint64_t)(timeout_ms * 1e6) : 0;
    }
}

static void end(sw_engine *e) {
    if (--e->depth == 0) e->deadline_ns = 0;
}

/// Writes the pending exception to err and classifies it.
static int fail(sw_engine *e, char *err, size_t errlen) {
    JSContext *ctx = e->ctx;
    JSValue exc = JS_GetException(ctx);
    const char *msg = NULL;
    const char *st = NULL;
    JSValue stack = JS_UNDEFINED, message = JS_UNDEFINED;
    if (JS_IsError(exc)) {
        message = JS_GetPropertyStr(ctx, exc, "message");
        if (!JS_IsException(message) && !JS_IsUndefined(message)) msg = JS_ToCString(ctx, message);
        stack = JS_GetPropertyStr(ctx, exc, "stack");
        if (JS_IsString(stack)) st = JS_ToCString(ctx, stack);
    }
    if (!msg) msg = JS_ToCString(ctx, exc);
    if (JS_HasException(ctx)) JS_FreeValue(ctx, JS_GetException(ctx));
    int code = SW_ERR_EXCEPTION;
    if (e->interrupted) code = SW_ERR_TIMEOUT;
    else if (!msg || (JS_IsError(exc) && strcmp(msg, "out of memory") == 0)) code = SW_ERR_OOM;
    if (err && errlen) snprintf(err, errlen, "%s", msg ? msg : "out of memory");
    snprintf(e->last_stack, sizeof(e->last_stack), "%s", st ? st : "");
    JS_FreeValue(ctx, message);
    // The exception (uncatchable or not) is consumed here, so nothing stays
    // pending in the context for the next call.
    if (st) JS_FreeCString(ctx, st);
    if (msg) JS_FreeCString(ctx, msg);
    JS_FreeValue(ctx, stack);
    JS_FreeValue(ctx, exc);
    return code;
}

/// Runs queued promise jobs. Returns -1 with a pending exception on failure.
static int run_jobs(sw_engine *e) {
    for (;;) {
        JSContext *jctx = NULL;
        int r = JS_ExecutePendingJob(e->rt, &jctx);
        if (r == 0) return 0;
        if (r < 0) return -1;
    }
}

/// Consumes `v`: encodes it into *result if requested, then runs jobs.
static int finish(sw_engine *e, JSValue v, uint8_t **result, size_t *result_len,
                  char *err, size_t errlen) {
    JSContext *ctx = e->ctx;
    if (result) *result = NULL;
    if (result_len) *result_len = 0;
    if (JS_IsException(v)) return fail(e, err, errlen);
    if (result) {
        buf b = {0};
        int rc = encode(ctx, &b, v, 0);
        JS_FreeValue(ctx, v);
        if (rc) { free(b.p); return fail(e, err, errlen); }
        if (b.oom) {
            free(b.p);
            if (err && errlen) snprintf(err, errlen, "out of memory (result)");
            return SW_ERR_OOM;
        }
        *result = b.p;
        if (result_len) *result_len = b.len;
    } else {
        JS_FreeValue(ctx, v);
    }
    if (run_jobs(e)) {
        if (result && *result) { free(*result); *result = NULL; }
        return fail(e, err, errlen);
    }
    if (err && errlen) err[0] = 0;
    return SW_OK;
}

int sw_engine_eval(sw_engine *e, const char *src, size_t len, const char *filename,
                   double timeout_ms, uint8_t **result, size_t *result_len,
                   char *err, size_t errlen) {
    if (!e || !src) return SW_ERR_ENGINE;
    begin(e, timeout_ms);
    JSValue v = JS_Eval(e->ctx, src, len, filename ? filename : "<eval>", JS_EVAL_TYPE_GLOBAL);
    int rc = finish(e, v, result, result_len, err, errlen);
    end(e);
    return rc;
}

int sw_engine_call_app(sw_engine *e, const char *method, const uint8_t *args,
                       size_t args_len, double timeout_ms, uint8_t **result,
                       size_t *result_len, char *err, size_t errlen) {
    if (!e || !method) return SW_ERR_ENGINE;
    JSContext *ctx = e->ctx;
    begin(e, timeout_ms);
    JSValue r = JS_UNDEFINED;
    JSValue global = JS_GetGlobalObject(ctx);
    JSValue app = JS_GetPropertyStr(ctx, global, "__app");
    JS_FreeValue(ctx, global);
    JSValue fn = JS_UNDEFINED, argv_arr = JS_UNDEFINED;
    JSValue *argv = NULL;
    int64_t argc = 0;
    if (JS_IsException(app)) { r = JS_EXCEPTION; goto done; }
    if (!JS_IsObject(app)) { r = JS_ThrowTypeError(ctx, "globalThis.__app is not set"); goto done; }
    fn = JS_GetPropertyStr(ctx, app, method);
    if (JS_IsException(fn)) { r = JS_EXCEPTION; goto done; }
    if (!JS_IsFunction(ctx, fn)) { r = JS_ThrowTypeError(ctx, "__app.%s is not a function", method); goto done; }
    if (args && args_len) {
        reader rd = { args, args_len, 0 };
        argv_arr = decode(ctx, &rd, 0);
        if (JS_IsException(argv_arr)) { r = JS_EXCEPTION; goto done; }
        if (!JS_IsArray(argv_arr)) { r = JS_ThrowTypeError(ctx, "host args are not an array"); goto done; }
        if (JS_GetLength(ctx, argv_arr, &argc)) { r = JS_EXCEPTION; goto done; }
        if (argc > 0) {
            argv = calloc((size_t)argc, sizeof(JSValue));
            if (!argv) { r = JS_ThrowOutOfMemory(ctx); goto done; }
            for (int64_t i = 0; i < argc; i++) argv[i] = JS_GetPropertyUint32(ctx, argv_arr, (uint32_t)i);
        }
    }
    r = JS_Call(ctx, fn, app, (int)argc, argv);
done:
    if (argv) {
        for (int64_t i = 0; i < argc; i++) JS_FreeValue(ctx, argv[i]);
        free(argv);
    }
    JS_FreeValue(ctx, argv_arr);
    JS_FreeValue(ctx, fn);
    JS_FreeValue(ctx, app);
    int rc = finish(e, r, result, result_len, err, errlen);
    end(e);
    return rc;
}

int sw_engine_gc(sw_engine *e, double timeout_ms, char *err, size_t errlen) {
    if (!e) return SW_ERR_ENGINE;
    begin(e, timeout_ms);
    JS_RunGC(e->rt);
    int rc = SW_OK;
    if (run_jobs(e)) rc = fail(e, err, errlen);
    else if (err && errlen) err[0] = 0;
    end(e);
    return rc;
}

void sw_engine_memory(sw_engine *e, sw_mem_usage *out) {
    JSMemoryUsage u;
    JS_ComputeMemoryUsage(e->rt, &u);
    out->malloc_size = u.malloc_size;
    out->malloc_count = u.malloc_count;
    out->memory_used_size = u.memory_used_size;
    out->obj_count = u.obj_count;
    out->js_func_count = u.js_func_count;
    out->js_func_code_size = u.js_func_code_size;
}

const char *sw_quickjs_version(void) { return JS_GetVersion(); }
