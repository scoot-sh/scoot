/* popup-flood-many: the aggregate popup shape from
 * docs/backlog/core/popup-aggregate-pressure-cap.md.
 *
 * K connections hold M side-by-side xdg_popups each (M at or under the
 * per-client cap of 128), every popup hanging off its own connection's
 * window, so the global PopupManager tree holds K*M popups with no client
 * past its cap. Barriers separate the phases so each prints the server's
 * stall for it: every get_popup with no commits and a round trip (phase
 * "track": admit plus the tree insert), every first commit and a round
 * trip (phase "commit": the commit scan plus the initial configure walk),
 * every destroy and a round trip (phase "destroy": the xdg_popup
 * destructor scan).
 *
 * No buffers on the popups (the stall is in tracking, not drawing), one
 * small buffer on each window so it is properly mapped. If any client is
 * refused (disconnected) the run prints REFUSED with how many survived,
 * and the exit is 0 -- this measures; it asserts nothing.
 *
 * usage: popup-flood-many K M
 */
#define _POSIX_C_SOURCE 200809L
#include <poll.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/wait.h>
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

static int dead(struct wl_display *display) {
    return wl_display_get_error(display) != 0;
}

static int make_shm_buffer(struct globals *g, struct wl_surface *surface, int w, int h) {
    int stride = w * 4, size = stride * h;
    char name[] = "/tmp/popup-flood-many-XXXXXX";
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

/* One flood client: M uncommitted popups off its own window, then commit,
 * then destroy, reporting each phase on `report` and waiting for one "go"
 * byte on `barrier` between phases. Reports 'X' and exits 1 if refused. */
static int child_main(long m, int report, int barrier) {
    struct wl_display *display = wl_display_connect(NULL);
    if (!display) return 1;

    struct globals g = { 0 };
    struct wl_registry *registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &registry_listener, &g);
    wl_display_roundtrip(display);
    if (!g.compositor || !g.shm || !g.wm_base) return 1;
    xdg_wm_base_add_listener(g.wm_base, &wm_listener, NULL);

    struct wl_surface *win = wl_compositor_create_surface(g.compositor);
    struct xdg_surface *win_xdg = xdg_wm_base_get_xdg_surface(g.wm_base, win);
    xdg_surface_add_listener(win_xdg, &surface_listener, NULL);
    struct xdg_toplevel *win_top = xdg_surface_get_toplevel(win_xdg);
    xdg_toplevel_add_listener(win_top, &toplevel_listener, NULL);
    wl_surface_commit(win);
    while (!acked_toplevel) wl_display_dispatch(display);
    if (make_shm_buffer(&g, win, toplevel_w, toplevel_h) < 0) return 1;
    wl_display_roundtrip(display);
    if (dead(display)) { (void)!write(report, "X", 1); return 1; }

    struct wl_surface **surfaces = calloc(m, sizeof *surfaces);
    struct xdg_surface **xdgs = calloc(m, sizeof *xdgs);
    struct xdg_popup **popups = calloc(m, sizeof *popups);
    if (!surfaces || !xdgs || !popups) return 1;

    for (long i = 0; i < m; i++) {
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
    if (dead(display)) { (void)!write(report, "X", 1); return 1; }
    (void)!write(report, "T", 1);

    char go = 0;
    if (read(barrier, &go, 1) != 1) return 1;
    for (long i = 0; i < m; i++) wl_surface_commit(surfaces[i]);
    wl_display_roundtrip(display);
    if (dead(display)) { (void)!write(report, "X", 1); return 1; }
    (void)!write(report, "C", 1);

    if (read(barrier, &go, 1) != 1) return 1;
    for (long i = 0; i < m; i++) xdg_popup_destroy(popups[i]);
    wl_display_roundtrip(display);
    if (dead(display)) { (void)!write(report, "X", 1); return 1; }
    (void)!write(report, "D", 1);

    for (long i = 0; i < m; i++) {
        xdg_surface_destroy(xdgs[i]);
        wl_surface_destroy(surfaces[i]);
    }
    wl_display_roundtrip(display);
    return dead(display);
}

/* Read one phase byte from each of k children (any order); -1 on EOF. */
static int gather(int fd, long k, char want) {
    long got = 0, refused = 0;
    while (got + refused < k) {
        char c = 0;
        if (read(fd, &c, 1) != 1) return -1;
        if (c == want) got++;
        else refused++;
    }
    return (int)refused;
}

int main(int argc, char **argv) {
    if (argc != 3) { fprintf(stderr, "usage: popup-flood-many K M\n"); return 2; }
    long k = atol(argv[1]), m = atol(argv[2]);
    if (k <= 0 || k > 500 || m <= 0 || m > 100000) { fprintf(stderr, "bad K M\n"); return 2; }

    int report[2], barrier[2];
    if (pipe(report) < 0 || pipe(barrier) < 0) return 1;

    for (long i = 0; i < k; i++) {
        pid_t pid = fork();
        if (pid < 0) return 1;
        if (pid == 0) {
            close(report[0]);
            int rc = child_main(m, report[1], barrier[0]);
            _exit(rc);
        }
    }
    close(report[1]);

    double t0 = now_ms();
    int refused = gather(report[0], k, 'T');
    double t1 = now_ms();
    if (refused < 0) { printf("STALL k=%ld m=%ld phase=track\n", k, m); return 0; }
    for (long i = 0; i < k; i++) (void)!write(barrier[1], "g", 1);
    refused += gather(report[0], k, 'C');
    double t2 = now_ms();
    if (refused < 0) { printf("STALL k=%ld m=%ld phase=commit\n", k, m); return 0; }
    for (long i = 0; i < k; i++) (void)!write(barrier[1], "g", 1);
    refused += gather(report[0], k, 'D');
    double t3 = now_ms();

    long alive = 0;
    int status = 0;
    for (long i = 0; i < k; i++) {
        if (waitpid(-1, &status, 0) < 0) break;
        if (WIFEXITED(status) && WEXITSTATUS(status) == 0) alive++;
    }
    if (refused > 0 || alive != k)
        printf("REFUSED k=%ld m=%ld alive=%ld track=%.1fms commit=%.1fms destroy=%.1fms\n",
               k, m, alive, t1 - t0, t2 - t1, t3 - t2);
    else
        printf("ADMITTED k=%ld m=%ld total=%ld track=%.1fms commit=%.1fms destroy=%.1fms\n",
               k, m, k * m, t1 - t0, t2 - t1, t3 - t2);
    return 0;
}
