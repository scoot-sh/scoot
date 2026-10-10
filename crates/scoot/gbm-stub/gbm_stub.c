/*
 * Static libgbm ABI stub for the `runtime-gbm` Cargo feature (spike).
 *
 * The problem it solves: Smithay's `backend_gbm` (via the `gbm` crate's
 * `gbm-sys`) carries `#[link(name = "gbm")]`, so any binary with the scanout
 * tier compiled in has libgbm in DT_NEEDED and will not start where that
 * library is absent. Linking this static archive satisfies `-lgbm` without
 * emitting DT_NEEDED (proven: `readelf -d` shows only libc/libgcc_s, with
 * and without a system libgbm present), and each symbol forwards to the
 * real libgbm, loaded by dlopen on first use, only when it is actually
 * called. With no libgbm on the box every entry point fails closed
 * (NULL/-1 plus ENOSYS, void ones a no-op), which is exactly what scoot's
 * existing fallbacks already handle: `tty::try_scanout` warns and keeps
 * dumb buffers, `nested::gpu::open_allocator` keeps read-back.
 *
 * Coverage: every function of the libgbm ABI the pinned `gbm` crate
 * (0.18.0) can call -- 36 entry points, enumerated from its sources
 * (`ffi::gbm_*` references). Types need no stubs (header-only in the Rust
 * bindings). `gbm_format_get_name` and `gbm_bo_get_device` are deliberately
 * absent: the pinned crate calls neither, so a future crate version that
 * does fails the link loudly at build time instead of silently missing at
 * runtime. Recheck this list on any `gbm` version bump.
 *
 * `scoot_gbm_real_loaded` is the spike's own probe (1: forwarding to a
 * real libgbm, 0: fail-closed): it lets a test prove which implementation
 * answered without needing a DRM device.
 *
 * Threading: the `dlopen` handle is resolved once per process under
 * `pthread_once`; every entry point looks its symbol up per call. A per-call
 * `dlsym` is a hash lookup, and none of these runs on a per-frame hot path
 * (allocation/import/query at startup, resize and buffer import), so caching
 * function pointers would buy nothing and only add shared state. A
 * *transient* `dlopen` failure pins the process to fail-closed for its life
 * (documented, not retried: GBM availability does not change under a
 * running compositor often enough to matter, and every caller already
 * treats refusal as a decision, not a retryable event).
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <pthread.h>
#include <stddef.h>
#include <stdint.h>

struct gbm_device;
struct gbm_bo;
struct gbm_surface;

/* Same layout as gbm.h's (every member at offset 0, 8 bytes total), so a
 * zeroed value returned fail-closed and an 8-byte value passed through are
 * both ABI-identical. Only ever constructed here as all-zero. */
union gbm_bo_handle {
    void *ptr;
    int32_t s32;
    uint32_t u32;
    int64_t s64;
    uint64_t u64;
};

static void *gbm_real_handle = NULL;
static pthread_once_t gbm_real_once = PTHREAD_ONCE_INIT;

static void gbm_real_init(void) {
    /* Bare soname: resolved through LD_LIBRARY_PATH / the binary's
     * RUNPATH / the loader cache / default dirs. This archive is
     * linked statically and exports no SONAME, so it can never
     * resolve to itself here. */
    gbm_real_handle = dlopen("libgbm.so.1", RTLD_NOW | RTLD_LOCAL);
}

static void *gbm_real(void) {
    pthread_once(&gbm_real_once, gbm_real_init);
    return gbm_real_handle;
}

int scoot_gbm_real_loaded(void) {
    return gbm_real() != NULL;
}

/* One entry point's address in the real library, or NULL when there is no
 * library (or it lacks the symbol -- an older libgbm under a newer stub:
 * just that entry fails closed while its siblings forward). */
static void *gbm_sym(const char *name) {
    void *handle = gbm_real();
    if (!handle)
        return NULL;
    return dlsym(handle, name);
}

#define GBM_PTR(name, ret, params, args) \
    ret gbm_##name params { \
        ret (*f) params = (ret (*) params)gbm_sym("gbm_" #name); \
        if (!f) { \
            errno = ENOSYS; \
            return NULL; \
        } \
        return f args; \
    }

#define GBM_INT(name, params, args) \
    int gbm_##name params { \
        int (*f) params = (int (*) params)gbm_sym("gbm_" #name); \
        if (!f) { \
            errno = ENOSYS; \
            return -1; \
        } \
        return f args; \
    }

#define GBM_U32(name, params, args) \
    uint32_t gbm_##name params { \
        uint32_t (*f) params = (uint32_t (*) params)gbm_sym("gbm_" #name); \
        if (!f) \
            return 0; \
        return f args; \
    }

#define GBM_U64(name, params, args) \
    uint64_t gbm_##name params { \
        uint64_t (*f) params = (uint64_t (*) params)gbm_sym("gbm_" #name); \
        if (!f) \
            return 0; \
        return f args; \
    }

#define GBM_VOID(name, params, args, fret, fparams) \
    void gbm_##name params { \
        fret (*f) fparams = (fret (*) fparams)gbm_sym("gbm_" #name); \
        if (f) \
            f args; \
    }

#define GBM_HANDLE(name, params, args, fret, fparams) \
    fret gbm_##name params { \
        fret (*f) fparams = (fret (*) fparams)gbm_sym("gbm_" #name); \
        if (!f) { \
            union gbm_bo_handle zero = { .ptr = NULL }; \
            return zero; \
        } \
        return f args; \
    }

/* Device */
GBM_PTR(create_device, struct gbm_device *,
        (int fd), (fd))
GBM_INT(device_get_fd, (struct gbm_device *gbm), (gbm))
GBM_PTR(device_get_backend_name, const char *,
        (struct gbm_device *gbm), (gbm))
GBM_INT(device_is_format_supported,
        (struct gbm_device *gbm, uint32_t format, uint32_t flags),
        (gbm, format, flags))
GBM_INT(device_get_format_modifier_plane_count,
        (struct gbm_device *gbm, uint32_t format, uint64_t modifier),
        (gbm, format, modifier))
GBM_VOID(device_destroy, (struct gbm_device *gbm), (gbm),
         void, (struct gbm_device *))

/* Buffer objects */
GBM_PTR(bo_create,
        struct gbm_bo *,
        (struct gbm_device *gbm, uint32_t width, uint32_t height,
         uint32_t format, uint32_t flags),
        (gbm, width, height, format, flags))
GBM_PTR(bo_create_with_modifiers,
        struct gbm_bo *,
        (struct gbm_device *gbm, uint32_t width, uint32_t height,
         uint32_t format, const uint64_t *modifiers,
         const unsigned int count),
        (gbm, width, height, format, modifiers, count))
GBM_PTR(bo_create_with_modifiers2,
        struct gbm_bo *,
        (struct gbm_device *gbm, uint32_t width, uint32_t height,
         uint32_t format, const uint64_t *modifiers,
         const unsigned int count, uint32_t flags),
        (gbm, width, height, format, modifiers, count, flags))
GBM_PTR(bo_import,
        struct gbm_bo *,
        (struct gbm_device *gbm, uint32_t type, void *buffer,
         uint32_t flags),
        (gbm, type, buffer, flags))
GBM_PTR(bo_map,
        void *,
        (struct gbm_bo *bo, uint32_t x, uint32_t y, uint32_t width,
         uint32_t height, uint32_t flags, uint32_t *stride,
         void **map_data),
        (bo, x, y, width, height, flags, stride, map_data))
GBM_VOID(bo_unmap, (struct gbm_bo *bo, void *map_data), (bo, map_data),
         void, (struct gbm_bo *, void *))
GBM_U32(bo_get_width, (struct gbm_bo *bo), (bo))
GBM_U32(bo_get_height, (struct gbm_bo *bo), (bo))
GBM_U32(bo_get_stride, (struct gbm_bo *bo), (bo))
GBM_U32(bo_get_stride_for_plane, (struct gbm_bo *bo, int plane), (bo, plane))
GBM_U32(bo_get_format, (struct gbm_bo *bo), (bo))
GBM_U32(bo_get_bpp, (struct gbm_bo *bo), (bo))
GBM_U32(bo_get_offset, (struct gbm_bo *bo, int plane), (bo, plane))
GBM_U64(bo_get_modifier, (struct gbm_bo *bo), (bo))
GBM_INT(bo_get_plane_count, (struct gbm_bo *bo), (bo))
GBM_INT(bo_get_fd, (struct gbm_bo *bo), (bo))
GBM_INT(bo_get_fd_for_plane, (struct gbm_bo *bo, int plane), (bo, plane))
GBM_INT(bo_write, (struct gbm_bo *bo, const void *buf, size_t count),
        (bo, buf, count))
GBM_HANDLE(bo_get_handle, (struct gbm_bo *bo), (bo),
           union gbm_bo_handle, (struct gbm_bo *))
GBM_HANDLE(bo_get_handle_for_plane, (struct gbm_bo *bo, int plane),
           (bo, plane),
           union gbm_bo_handle, (struct gbm_bo *, int))
GBM_VOID(bo_set_user_data,
         (struct gbm_bo *bo, void *data,
          void (*destroy_user_data)(struct gbm_bo *, void *)),
         (bo, data, destroy_user_data),
         void, (struct gbm_bo *, void *,
                void (*)(struct gbm_bo *, void *)))
GBM_PTR(bo_get_user_data, void *, (struct gbm_bo *bo), (bo))
GBM_VOID(bo_destroy, (struct gbm_bo *bo), (bo),
         void, (struct gbm_bo *))

/* Surfaces */
GBM_PTR(surface_create,
        struct gbm_surface *,
        (struct gbm_device *gbm, uint32_t width, uint32_t height,
         uint32_t format, uint32_t flags),
        (gbm, width, height, format, flags))
GBM_PTR(surface_create_with_modifiers,
        struct gbm_surface *,
        (struct gbm_device *gbm, uint32_t width, uint32_t height,
         uint32_t format, const uint64_t *modifiers,
         const unsigned int count),
        (gbm, width, height, format, modifiers, count))
GBM_PTR(surface_create_with_modifiers2,
        struct gbm_surface *,
        (struct gbm_device *gbm, uint32_t width, uint32_t height,
         uint32_t format, const uint64_t *modifiers,
         const unsigned int count, uint32_t flags),
        (gbm, width, height, format, modifiers, count, flags))
GBM_PTR(surface_lock_front_buffer, struct gbm_bo *,
        (struct gbm_surface *surface), (surface))
GBM_VOID(surface_release_buffer,
         (struct gbm_surface *surface, struct gbm_bo *bo), (surface, bo),
         void, (struct gbm_surface *, struct gbm_bo *))
GBM_INT(surface_has_free_buffers, (struct gbm_surface *surface), (surface))
GBM_VOID(surface_destroy, (struct gbm_surface *surface), (surface),
         void, (struct gbm_surface *))
