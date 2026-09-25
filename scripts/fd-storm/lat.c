/* fd-storm: round-trip latency monitor, wl_display_roundtrip every 10 ms
 * for SECS s; prints the worst wait. From the PR #241 review.
 * usage: lat SECS */
#include <wayland-client.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>
#include <unistd.h>
static double now(void){struct timespec t;clock_gettime(CLOCK_MONOTONIC,&t);return t.tv_sec*1e3+t.tv_nsec/1e6;}
int main(int argc, char **argv) {
	double secs = atof(argv[1]);
	struct wl_display *d = wl_display_connect(NULL); if (!d) { perror("connect"); return 2; }
	printf("LAT ready\n"); fflush(stdout);
	double start = now(), max = 0, sum = 0; int n = 0, over50 = 0, over500 = 0;
	while (now() - start < secs * 1000) {
		double t = now();
		if (wl_display_roundtrip(d) < 0) { printf("LAT roundtrip failed\n"); break; }
		double dt = now() - t; sum += dt; n++; if (dt > max) max = dt; if (dt > 50) over50++; if (dt > 500) over500++;
		if (dt > 100) { printf("LAT stall %.0f ms at t=%.1fs\n", dt, (t-start)/1000); fflush(stdout); }
		usleep(10000);
	}
	printf("LAT n=%d mean=%.2fms max=%.1fms over50=%d over500=%d\n", n, sum/n, max, over50, over500);
	return 0;
}
