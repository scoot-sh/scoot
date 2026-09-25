// fd-storm: park fds in scoot's received-fd queue. N wl_display.sync
// requests (no fd argument), each carrying PER SCM_RIGHTS copies of one
// /dev/null fd that no request claims; hold HOLD s, then print any
// wl_display.error the compositor sent. From the PR #236/#241 reviews.
// usage: park N HOLD [PER]
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>
int main(int argc, char **argv) {
  int n = atoi(argv[1]), hold = atoi(argv[2]);
  int per = argc > 3 ? atoi(argv[3]) : 28;
  if (per < 1 || per > 28) { fprintf(stderr, "PER must be 1..28\n"); return 2; }
  struct sockaddr_un a = {.sun_family = AF_UNIX};
  snprintf(a.sun_path, sizeof a.sun_path, "%s/%s", getenv("XDG_RUNTIME_DIR"), getenv("WAYLAND_DISPLAY"));
  int s = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
  if (connect(s, (struct sockaddr *)&a, sizeof a)) { perror("connect"); return 1; }
  int fd = open("/dev/null", O_RDONLY | O_CLOEXEC);
  int sent = 0;
  for (int i = 0; i < n; i++) {
    uint32_t msg[3] = {1, (12u << 16) | 0, 2 + i}; // wl_display.sync(new_id)
    struct iovec iov = {msg, sizeof msg};
    char cbuf[CMSG_SPACE(sizeof(int) * 28)];
    memset(cbuf, 0, sizeof cbuf);
    struct msghdr m = {.msg_iov = &iov, .msg_iovlen = 1, .msg_control = cbuf, .msg_controllen = CMSG_SPACE(sizeof(int) * per)};
    struct cmsghdr *c = CMSG_FIRSTHDR(&m);
    c->cmsg_level = SOL_SOCKET; c->cmsg_type = SCM_RIGHTS; c->cmsg_len = CMSG_LEN(sizeof(int) * per);
    int *fds = (int *)CMSG_DATA(c);
    for (int j = 0; j < per; j++) fds[j] = fd;
    if (sendmsg(s, &m, MSG_NOSIGNAL) < 0) { perror("sendmsg"); printf("STUFFED-FAIL at %d\n", i); fflush(stdout); break; }
    sent++;
    usleep(2000);
  }
  printf("STUFFED %d msgs x %d fds = %d fds\n", sent, per, sent * per); fflush(stdout);
  sleep(hold);
  // Drain everything the server sent; look for wl_display.error.
  static unsigned char b[1 << 16]; size_t got = 0; ssize_t r;
  int eof = 0;
  while (got < sizeof b && (r = recv(s, b + got, sizeof b - got, MSG_DONTWAIT)) > 0) got += r;
  if (r == 0) eof = 1;
  for (size_t off = 0; off + 8 <= got;) {
    uint32_t obj, w2; memcpy(&obj, b + off, 4); memcpy(&w2, b + off + 4, 4);
    uint32_t size = w2 >> 16, op = w2 & 0xffff;
    if (size < 8 || off + size > got) break;
    if (obj == 1 && op == 0 && size >= 20) {
      uint32_t eobj, code, len; memcpy(&eobj, b + off + 8, 4); memcpy(&code, b + off + 12, 4); memcpy(&len, b + off + 16, 4);
      printf("WL_DISPLAY_ERROR object=%u code=%u message=\"%.*s\"\n", eobj, code, (int)(len ? len - 1 : 0), (char *)(b + off + 20)); fflush(stdout);
    }
    off += size;
  }
  printf("END connected=%s bytes_read=%zu\n", eof ? "no(EOF)" : "yes-or-eagain", got); fflush(stdout);
  return 0;
}
