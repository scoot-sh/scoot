/* fd-storm: open N connections to the Wayland socket as fast as possible and
 * hold them HOLD s. From the PR #241 review (the accept-storm freeze).
 * usage: storm N HOLD */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <sys/resource.h>
#include <unistd.h>
#include <time.h>
int main(int argc, char **argv) {
	int n = atoi(argv[1]), hold = atoi(argv[2]);
	struct rlimit rl; getrlimit(RLIMIT_NOFILE, &rl); rl.rlim_cur = rl.rlim_max; setrlimit(RLIMIT_NOFILE, &rl);
	struct sockaddr_un a = {.sun_family = AF_UNIX};
	snprintf(a.sun_path, sizeof a.sun_path, "%s/%s", getenv("XDG_RUNTIME_DIR"), getenv("WAYLAND_DISPLAY"));
	struct timespec t0, t1; clock_gettime(CLOCK_MONOTONIC, &t0);
	int ok = 0;
	for (int i = 0; i < n; i++) {
		int s = socket(AF_UNIX, SOCK_STREAM | SOCK_NONBLOCK, 0);
		if (s < 0) { perror("socket"); break; }
		if (connect(s, (struct sockaddr *)&a, sizeof a) == 0) ok++; else { close(s); }
	}
	clock_gettime(CLOCK_MONOTONIC, &t1);
	printf("STORM connected=%d of %d in %.1f ms\n", ok, n, (t1.tv_sec-t0.tv_sec)*1e3+(t1.tv_nsec-t0.tv_nsec)/1e6); fflush(stdout);
	sleep(hold);
	return 0;
}
