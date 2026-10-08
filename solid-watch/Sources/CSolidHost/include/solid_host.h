#ifndef SOLID_HOST_H
#define SOLID_HOST_H

// C glue between QuickJS and Swift. Swift never touches a JSValue (its
// layout differs between arm64 and arm64_32); values cross the boundary in
// the small tagged binary encoding below, in both directions.
//
// Value encoding (little-endian):
//   0x00 undefined          0x01 null
//   0x02 false              0x03 true
//   0x04 number   f64
//   0x05 string   u32 len, UTF-8 bytes
//   0x06 bytes    u32 len, bytes          (Uint8Array / ArrayBuffer)
//   0x07 array    u32 count, values
//   0x08 object   u32 count, (u32 len, UTF-8 key, value)*   (key order kept)
//   0x09 function                         (JS -> host only, no payload)
//   0x0A json     u32 len, UTF-8 JSON text (host -> JS only, JSON.parse'd)

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

enum {
    SW_V_UNDEFINED = 0x00,
    SW_V_NULL = 0x01,
    SW_V_FALSE = 0x02,
    SW_V_TRUE = 0x03,
    SW_V_NUMBER = 0x04,
    SW_V_STRING = 0x05,
    SW_V_BYTES = 0x06,
    SW_V_ARRAY = 0x07,
    SW_V_OBJECT = 0x08,
    SW_V_FUNCTION = 0x09,
    SW_V_JSON = 0x0A,
};

/// Result codes of the sw_engine_* calls.
enum {
    SW_OK = 0,
    SW_ERR_EXCEPTION = 1,   ///< a JS exception (message + stack in `err`)
    SW_ERR_TIMEOUT = 2,     ///< the call ran past its deadline and was interrupted
    SW_ERR_OOM = 3,         ///< the runtime hit its memory limit
    SW_ERR_ENGINE = 4,      ///< no engine / bad arguments from the host
};

/// The `__host` functions a bundle calls. `user` is passed back unchanged.
/// Mutation callbacks return 0 on success; on failure they call
/// sw_engine_set_host_error() and return nonzero, and the JS call throws a
/// TypeError with that message.
typedef struct sw_host_callbacks {
    void *user;
    void (*log)(void *user, const char *msg, size_t len);
    /// Returns the new tag (> 0), or <= 0 on failure.
    int32_t (*create_node)(void *user, const char *type, size_t len);
    int32_t (*create_text)(void *user, const char *text, size_t len);
    /// `value` is one encoded value.
    int (*set_prop)(void *user, int32_t tag, const char *key, size_t keylen,
                    const uint8_t *value, size_t valuelen);
    /// `before` is 0 to append.
    int (*insert)(void *user, int32_t parent, int32_t child, int32_t before);
    int (*remove)(void *user, int32_t parent, int32_t child);
    int (*commit)(void *user);
    /// `syncClock(deviceNowMs)`: the state being applied was captured at
    /// this device time (README: Time).
    int (*sync_clock)(void *user, double device_now_ms);
    /// `release(tag)`: JS can no longer reach `tag` (from the renderer's
    /// FinalizationRegistry, as a pending job after a GC). Not recorded.
    int (*release)(void *user, int32_t tag);
    /// While recording is on (sw_engine_set_recording), each `__host` call
    /// is reported as one `JSON.stringify([op, ...args])` line, made by the
    /// engine itself so it matches a JS recording host byte for byte:
    /// ["createNode",tag,type] ["createText",tag,text] ["setProp",tag,key,value]
    /// ["insert",parent,child,before] ["remove",parent,child] ["commit"]
    /// ["syncClock",deviceNowMs].
    void (*record)(void *user, const char *line, size_t len);
} sw_host_callbacks;

typedef struct sw_engine sw_engine;

typedef struct sw_engine_options {
    size_t memory_limit;    ///< bytes, 0 = unlimited
    size_t max_stack_size;  ///< bytes, 0 = QuickJS default
} sw_engine_options;

typedef struct sw_mem_usage {
    int64_t malloc_size;
    int64_t malloc_count;
    int64_t memory_used_size;
    int64_t obj_count;
    int64_t js_func_count;
    int64_t js_func_code_size;
} sw_mem_usage;

/// A runtime + context with `globalThis.__host` and a minimal `console`
/// (log/info/warn/error -> the log callback) installed.
sw_engine *sw_engine_new(const sw_host_callbacks *cb, const sw_engine_options *opt);
void sw_engine_free(sw_engine *e);

/// Message for the next failing callback return (copied).
void sw_engine_set_host_error(sw_engine *e, const char *msg);

void sw_engine_set_recording(sw_engine *e, int on);

/// `JSON.stringify(value)` of one encoded value, computed by the engine.
/// Returns a malloc'd NUL-terminated string (free with sw_free), or NULL.
char *sw_engine_stringify(sw_engine *e, const uint8_t *value, size_t len);

/// The JS stack of the last failed call ("" if none).
const char *sw_engine_last_stack(sw_engine *e);

/// Evaluates global script `src` (`len` bytes; src[len] must be NUL).
/// `timeout_ms` <= 0 means no deadline. On SW_OK, if `result` is non-NULL,
/// *result receives the encoded completion value (free with sw_free). On
/// failure, `err` receives the error's message (`e.message` for an Error,
/// else `String(e)`); the stack is in sw_engine_last_stack().
int sw_engine_eval(sw_engine *e, const char *src, size_t len, const char *filename,
                   double timeout_ms, uint8_t **result, size_t *result_len,
                   char *err, size_t errlen);

/// Calls `globalThis.__app[method](...args)` where `args` is one encoded
/// array value. Pending jobs (promises) run afterwards, inside the deadline.
int sw_engine_call_app(sw_engine *e, const char *method, const uint8_t *args,
                       size_t args_len, double timeout_ms, uint8_t **result,
                       size_t *result_len, char *err, size_t errlen);

void sw_free(void *p);
/// Runs a garbage collection, then the pending jobs it queued (finalizers:
/// `release` calls). Returns SW_OK or an error code like the calls above.
int sw_engine_gc(sw_engine *e, double timeout_ms, char *err, size_t errlen);
void sw_engine_memory(sw_engine *e, sw_mem_usage *out);
const char *sw_quickjs_version(void);

#ifdef __cplusplus
}
#endif

#endif
