/* popup-flood: open N side-by-side xdg_popups of one window, time it.
 *
 * One round trip carries every get_popup with no commits (phase 1: the
 * server dispatches all N trackings -- admit plus the tree insert), a
 * second every first commit (phase 2: the commit scan plus the initial
 * configure walk), and a third destroys them all again (phase 3: the
 * xdg_popup destructor scan). Each round trip waits out the server's
 * dispatch, so each phase time is the server's stall for it, plus the
 * client's own send/receive.
 *
 * No buffers: popup surfaces commit empty (the stall is in tracking, not
 * drawing), and 5000 shm pools would trip the client's own 512-pool bound
 * long before the popup count mattered. The window gets one small buffer so
 * it is properly mapped. Past a per-client popup cap the client is refused
 * instead: the wl_display error is printed and the exit is 0, bench-style
 * (this measures; it asserts nothing).
 *
 * usage: popup-flood N
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
    struct wl_shm *shm;
    struct xdg_wm_base *wm_base;
};

static void registry_global(void *data, struct wl_registry *registry,
                            uint32_t name, const char *interface, uint32_t version) {
    struct globals *g = data;
    if (strcmp(interface, "wl_compositor") == 0) {
        g->compositor = wl_registry_bind(registry, name, &wl_compositor_interface, 4);
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
static uint32_t toplevel_serial = 0;
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
    toplevel_serial = serial;
    acked_toplevel = 1;
    xdg_surface_ack_configure(surface, serial);
}

static const struct xdg_surface_listener surface_listener = { surface_configure };

/* Whether the server has killed us (a refusal disconnects the client). */
static int dead(struct wl_display *display) {
    return wl_display_get_error(display) != 0;
}

static int make_shm_buffer(struct globals *g, struct wl_surface *surface, int w, int h) {
    int stride = w * 4, size = stride * h;
    char name[] = "/tmp/popup-flood-XXXXXX";
    int fd = mkstemp(name);
    if (fd < 0) return -1;
    unlink(name);
    if (ftruncate(fd, size) < 0) { close(fd); return -1; }
    void *px = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (px == MAP_FAILED) { close(fd); return -1; }
    memset(px, 0x80, size);
    munmap(px, size);
    struct wl_shm_pool *pool = wl_shm_create_pool(g->shm, fd, size);
    struct wl_buffer *buf = wl_shm_pool_create_buffer(pool, 0, w, h, stride,
                                                      WL_SHM_FORMAT_ARGB8888);
    wl_shm_pool_destroy(pool);
    close(fd);
    if (!buf) return -1;
    wl_surface_attach(surface, buf, 0, 0);
    wl_surface_damage(surface, 0, 0, w, h);
    wl_surface_commit(surface);
    return 0;
}

int main(int argc, char **argv) {
    if (argc != 2) { fprintf(stderr, "usage: popup-flood N\n"); return 2; }
    long n = atol(argv[1]);
    if (n <= 0 || n > 100000) { fprintf(stderr, "bad N\n"); return 2; }

    struct wl_display *display = wl_display_connect(NULL);
    if (!display) { fprintf(stderr, "no display\n"); return 1; }

    struct globals g = { 0 };
    struct wl_registry *registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &registry_listener, &g);
    wl_display_roundtrip(display);
    if (!g.compositor || !g.shm || !g.wm_base) {
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
    if (make_shm_buffer(&g, win, toplevel_w, toplevel_h) < 0) {
        fprintf(stderr, "window buffer failed\n");
        return 1;
    }
    wl_display_roundtrip(display);

    struct wl_surface **surfaces = calloc(n, sizeof *surfaces);
    struct xdg_surface **xdgs = calloc(n, sizeof *xdgs);
    struct xdg_popup **popups = calloc(n, sizeof *popups);
    if (!surfaces || !xdgs || !popups) return 1;

    /* Phase 1: every get_popup, no commits. The round trip waits out the
     * server dispatching all N trackings (admit plus the tree insert). */
    double t0 = now_ms();
    for (long i = 0; i < n; i++) {
        surfaces[i] = wl_compositor_create_surface(g.compositor);
        xdgs[i] = xdg_wm_base_get_xdg_surface(g.wm_base, surfaces[i]);
        struct xdg_positioner *pos = xdg_wm_base_create_positioner(g.wm_base);
        xdg_positioner_set_size(pos, 8, 8);
        xdg_positioner_set_anchor_rect(pos, 1, 1, 1, 1);
        xdg_positioner_set_anchor(pos, XDG_POSITIONER_ANCHOR_TOP_LEFT);
        xdg_positioner_set_gravity(pos, XDG_POSITIONER_GRAVITY_BOTTOM_RIGHT);
        popups[i] = xdg_surface_get_popup(xdgs[i], win_xdg, pos);
        xdg_positioner_destroy(pos);
    }
    wl_display_roundtrip(display);
    double t1 = now_ms();
    if (dead(display)) {
        printf("REFUSED n=%ld phase=track after=%.1fms\n", n, t1 - t0);
        return 0;
    }

    /* Phase 2: every first commit. The round trip waits out the server
     * dispatching all N commits (the commit scan plus the initial
     * configure walk) and sending every configure. */
    for (long i = 0; i < n; i++) wl_surface_commit(surfaces[i]);
    wl_display_roundtrip(display);
    double t2 = now_ms();
    if (dead(display)) {
        printf("REFUSED n=%ld phase=commit after=%.1fms\n", n, t2 - t0);
        return 0;
    }

    /* No ack round trip is needed for the timing: no popup ever attaches a
     * buffer, so no ack is owed. The configure backlog is simply read and
     * dropped by the round trips above and below. */

    /* Phase 3: destroy them all again (the destructor scan). */
    for (long i = 0; i < n; i++) xdg_popup_destroy(popups[i]);
    wl_display_roundtrip(display);
    double t3 = now_ms();
    if (dead(display)) {
        printf("REFUSED n=%ld phase=destroy after=%.1fms\n", n, t3 - t0);
        return 0;
    }

    for (long i = 0; i < n; i++) {
        xdg_surface_destroy(xdgs[i]);
        wl_surface_destroy(surfaces[i]);
    }
    wl_display_roundtrip(display);
    double t4 = now_ms();

    printf("ADMITTED n=%ld track=%.1fms commit=%.1fms destroy=%.1fms teardown=%.1fms\n",
           n, t1 - t0, t2 - t1, t3 - t2, t4 - t3);
    return 0;
}
