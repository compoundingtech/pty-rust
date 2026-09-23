#include <arpa/inet.h>
#include <libproc.h>
#include <netinet/in.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <sys/proc_info.h>
#include <sys/socket.h>

/*
 * Narrow Darwin libproc boundary for Rust. Return the owning requested pid,
 * zero for a complete negative observation, and -1 whenever the descriptor
 * table cannot be proved complete.
 */
int32_t pty_inspect_socket_owner_darwin(
  const char *local_text,
  uint16_t local_port,
  const char *foreign_text,
  uint16_t foreign_port,
  const int32_t *pids,
  size_t pid_count
) {
  int family = strchr(local_text, ':') == NULL ? AF_INET : AF_INET6;
  if ((strchr(foreign_text, ':') == NULL ? AF_INET : AF_INET6) != family) return -1;

  struct in6_addr local6;
  struct in6_addr foreign6;
  struct in_addr local4;
  struct in_addr foreign4;
  void *local_address = family == AF_INET ? (void *)&local4 : (void *)&local6;
  void *foreign_address = family == AF_INET ? (void *)&foreign4 : (void *)&foreign6;
  if (inet_pton(family, local_text, local_address) != 1 ||
      inet_pton(family, foreign_text, foreign_address) != 1) return -1;

  for (size_t arg = 0; arg < pid_count; arg++) {
    int32_t pid = pids[arg];
    if (pid <= 0) return -1;
    int bytes = proc_pidinfo(pid, PROC_PIDLISTFDS, 0, NULL, 0);
    if (bytes <= 0 || bytes % (int)sizeof(struct proc_fdinfo) != 0) return -1;
    int capacity = bytes + 32 * (int)sizeof(struct proc_fdinfo);
    struct proc_fdinfo *fds = calloc(1, (size_t)capacity);
    if (fds == NULL) return -1;
    int read_bytes = proc_pidinfo(pid, PROC_PIDLISTFDS, 0, fds, capacity);
    if (read_bytes <= 0 || read_bytes >= capacity || read_bytes % (int)sizeof(*fds) != 0) {
      free(fds);
      return -1;
    }

    int count = read_bytes / (int)sizeof(*fds);
    for (int index = 0; index < count; index++) {
      if (fds[index].proc_fdtype != PROX_FDTYPE_SOCKET) continue;
      struct socket_fdinfo socket_info;
      int socket_bytes = proc_pidfdinfo(
        pid,
        fds[index].proc_fd,
        PROC_PIDFDSOCKETINFO,
        &socket_info,
        (int)sizeof(socket_info)
      );
      if (socket_bytes != (int)sizeof(socket_info)) {
        free(fds);
        return -1;
      }
      if (socket_info.psi.soi_kind != SOCKINFO_TCP ||
          socket_info.psi.soi_protocol != IPPROTO_TCP ||
          socket_info.psi.soi_family != family ||
          socket_info.psi.soi_proto.pri_tcp.tcpsi_state != TSI_S_ESTABLISHED) continue;

      const struct in_sockinfo *info = &socket_info.psi.soi_proto.pri_tcp.tcpsi_ini;
      if (ntohs((uint16_t)info->insi_lport) != local_port ||
          ntohs((uint16_t)info->insi_fport) != foreign_port) continue;
      int address_match = family == AF_INET
        ? memcmp(&info->insi_laddr.ina_46.i46a_addr4, local_address, sizeof(struct in_addr)) == 0 &&
          memcmp(&info->insi_faddr.ina_46.i46a_addr4, foreign_address, sizeof(struct in_addr)) == 0
        : memcmp(&info->insi_laddr.ina_6, local_address, sizeof(struct in6_addr)) == 0 &&
          memcmp(&info->insi_faddr.ina_6, foreign_address, sizeof(struct in6_addr)) == 0;
      if (address_match) {
        free(fds);
        return pid;
      }
    }
    free(fds);
  }
  return 0;
}
