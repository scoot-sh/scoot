/* subsurface-flood: hang N 1-deep 4x4 sibling subsurfaces off one window, time it.
 *
 * The batch from docs/backlog/core/subsurface-count-quadratic.md: a client
 * maps a window, then in one batch creates N sibling subsurfaces of it --
 * each one level deep, with a 4x4 buffer, committed -- and commits the
 * window. The round trip waits out the server dispatching the whole batch,
 * so the printed time is the server's stall for it, plus the client's own
 * send/receive.
 *
 * Every subsurface attaches the same shared 4x4 shm buffer: the stall is in
 * per-commit window work, not drawing, and thousands of pools would trip
 * the client's own 512-pool bound long before the subsurface count
 * mattered. The window gets one small buffer so it is properly mapped.
 *
 * usage: subsurface-flood MODE N   (MODE is "desync" or "sync")
 */
#define _POSIX_C_SOURCE 200809L
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>
#include <wayland-client.h>
#include "xdg-shell-client-protocol.h"

static double now_ms(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec * 1000.0 + ts.tv_nsec / 1e6;
}

struct globals {
    struct wl_compositor *compositor;
    struct wl_subcompositor *subcompositor;
    struct wl_shm *shm;
    struct xdg_wm_base *wm_base;
};

static void registry_global(void *data, struct wl_registry *registry,
                            uint32_t name, const char *interface, uint32_t version) {
    struct globals *g = data;
    if (strcmp(interface, "wl_compositor") == 0) {
        g->compositor = wl_registry_bind(registry, name, &wl_compositor_interface, 4);
    } else if (strcmp(interface, "wl_subcompositor") == 0) {
        g->subcompositor = wl_registry_bind(registry, name, &wl_subcompositor_interface, 1);
    } else if (strcmp(interface, "wl_shm") == 0) {
        g->shm = wl_registry_bind(registry, name, &wl_shm_interface, 1);
    } else if (strcmp(interface, "xdg_wm_base") == 0) {
        g->wm_base = wl_registry_bind(registry, name, &xdg_wm_base_interface,
                                      version < 3 ? version : 3);
    }
}

static void registry_global_remove(void *data, struct wl_registry *registry, uint32_t name) {
    (void)data; (void)registry; (void)name;
}

static const struct wl_registry_listener registry_listener = {
    registry_global, registry_global_remove,
};

static void wm_ping(void *data, struct xdg_wm_base *wm_base, uint32_t serial) {
    (void)data;
    xdg_wm_base_pong(wm_base, serial);
}

static const struct xdg_wm_base_listener wm_listener = { wm_ping };

static int acked_toplevel = 0;
static int32_t toplevel_w = 0, toplevel_h = 0;

static void toplevel_configure(void *data, struct xdg_toplevel *toplevel,
                               int32_t w, int32_t h, struct wl_array *states) {
    (void)data; (void)toplevel; (void)states;
    toplevel_w = w > 0 ? w : 64;
    toplevel_h = h > 0 ? h : 64;
}

static void toplevel_close(void *data, struct xdg_toplevel *toplevel) {
    (void)data; (void)toplevel;
}

static const struct xdg_toplevel_listener toplevel_listener = {
    toplevel_configure, toplevel_close,
};

static void surface_configure(void *data, struct xdg_surface *surface, uint32_t serial) {
    (void)data;
    acked_toplevel = 1;
    xdg_surface_ack_configure(surface, serial);
}

static const struct xdg_surface_listener surface_listener = { surface_configure };

/* Whether the server has killed us (a refusal disconnects the client). */
static int dead(struct wl_display *display) {
    return wl_display_get_error(display) != 0;
}

/* One shm buffer of WxH, left attached to nothing; the caller attaches it
 * wherever it is needed. A single pool and buffer for the whole run. */
static struct wl_buffer *make_shm_buffer(struct globals *g, int w, int h) {
    int stride = w * 4, size = stride * h;
    char name[] = "/tmp/subsurface-flood-XXXXXX";
    int fd = mkstemp(name);
    if (fd < 0) return NULL;
    unlink(name);
    if (ftruncate(fd, size) < 0) { close(fd); return NULL; }
    void *px = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (px == MAP_FAILED) { close(fd); return NULL; }
    memset(px, 0x80, size);
    munmap(px, size);
    struct wl_shm_pool *pool = wl_shm_create_pool(g->shm, fd, size);
    struct wl_buffer *buf = wl_shm_pool_create_buffer(pool, 0, w, h, stride,
                                                      WL_SHM_FORMAT_ARGB8888);
    wl_shm_pool_destroy(pool);
    close(fd);
    return buf;
}

int main(int argc, char **argv) {
    if (argc != 3) { fprintf(stderr, "usage: subsurface-flood MODE N\n"); return 2; }
    int desync;
    if (strcmp(argv[1], "desync") == 0) desync = 1;
    else if (strcmp(argv[1], "sync") == 0) desync = 0;
    else { fprintf(stderr, "MODE is desync or sync\n"); return 2; }
    long n = atol(argv[2]);
    if (n <= 0 || n > 100000) { fprintf(stderr, "bad N\n"); return 2; }

    struct wl_display *display = wl_display_connect(NULL);
    if (!display) { fprintf(stderr, "no display\n"); return 1; }

    struct globals g = { 0 };
    struct wl_registry *registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &registry_listener, &g);
    wl_display_roundtrip(display);
    if (!g.compositor || !g.subcompositor || !g.shm || !g.wm_base) {
        fprintf(stderr, "missing globals\n");
        return 1;
    }
    xdg_wm_base_add_listener(g.wm_base, &wm_listener, NULL);

    /* One mapped window for everything to hang off. */
    struct wl_surface *win = wl_compositor_create_surface(g.compositor);
    struct xdg_surface *win_xdg = xdg_wm_base_get_xdg_surface(g.wm_base, win);
    xdg_surface_add_listener(win_xdg, &surface_listener, NULL);
    struct xdg_toplevel *win_top = xdg_surface_get_toplevel(win_xdg);
    xdg_toplevel_add_listener(win_top, &toplevel_listener, NULL);
    wl_surface_commit(win);
    while (!acked_toplevel) wl_display_dispatch(display);
    struct wl_buffer *win_buf = make_shm_buffer(&g, toplevel_w, toplevel_h);
    if (!win_buf) { fprintf(stderr, "window buffer failed\n"); return 1; }
    wl_surface_attach(win, win_buf, 0, 0);
    wl_surface_damage(win, 0, 0, toplevel_w, toplevel_h);
    wl_surface_commit(win);
    wl_display_roundtrip(display);

    /* The one 4x4 buffer every subsurface shares. */
    struct wl_buffer *sub_buf = make_shm_buffer(&g, 4, 4);
    if (!sub_buf) { fprintf(stderr, "subsurface buffer failed\n"); return 1; }

    struct wl_surface **surfaces = calloc(n, sizeof *surfaces);
    if (!surfaces) return 1;

    /* The batch: N 1-deep siblings, each committed, then the window commit.
     * Desynchronized commits apply at once; synchronized ones are cached
     * until the parent's commit, which is why that case is cheap. The round
     * trip waits out the server dispatching all of it. */
    double t0 = now_ms();
    for (long i = 0; i < n; i++) {
        surfaces[i] = wl_compositor_create_surface(g.compositor);
        struct wl_subsurface *sub =
            wl_subcompositor_get_subsurface(g.subcompositor, surfaces[i], win);
        if (desync) wl_subsurface_set_desync(sub);
        wl_surface_attach(surfaces[i], sub_buf, 0, 0);
        wl_surface_damage(surfaces[i], 0, 0, 4, 4);
        wl_surface_commit(surfaces[i]);
    }
    wl_surface_commit(win);
    wl_display_roundtrip(display);
    double t1 = now_ms();
    if (dead(display)) {
        printf("REFUSED mode=%s n=%ld after=%.1fms\n", argv[1], n, t1 - t0);
        return 0;
    }

    for (long i = 0; i < n; i++) wl_surface_destroy(surfaces[i]);
    free(surfaces);
    wl_display_roundtrip(display);
    double t2 = now_ms();

    printf("FLOODED mode=%s n=%ld batch=%.1fms teardown=%.1fms\n",
           argv[1], n, t1 - t0, t2 - t1);
    return 0;
}
