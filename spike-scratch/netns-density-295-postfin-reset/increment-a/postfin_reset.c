#define _GNU_SOURCE

#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <linux/tls.h>
#include <linux/inet_diag.h>
#include <linux/netlink.h>
#include <linux/sock_diag.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <signal.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#ifndef SOL_TLS
#define SOL_TLS 282
#endif

#ifndef TCP_ULP
#define TCP_ULP 31
#endif

#ifndef TCP_REPAIR
#define TCP_REPAIR 19
#endif

enum candidate {
    CANDIDATE_LINGER_ZERO,
    CANDIDATE_LINGER2_NEGATIVE,
    CANDIDATE_TCP_REPAIR,
    CANDIDATE_SOCK_DESTROY,
};

struct peer_outcome {
    int state_after_candidate;
    int so_error_after_candidate;
    int recv_result;
    int recv_errno;
};

static void die(const char *what)
{
    perror(what);
    exit(2);
}

static void write_exact(int fd, const void *buf, size_t len)
{
    const uint8_t *cursor = buf;
    while (len != 0) {
        ssize_t written = write(fd, cursor, len);
        if (written < 0)
            die("write");
        cursor += (size_t)written;
        len -= (size_t)written;
    }
}

static void read_exact(int fd, void *buf, size_t len)
{
    uint8_t *cursor = buf;
    while (len != 0) {
        ssize_t got = read(fd, cursor, len);
        if (got <= 0)
            die("read");
        cursor += (size_t)got;
        len -= (size_t)got;
    }
}

static void arm_ktls(int fd)
{
    static const char ulp[] = "tls";
    struct tls12_crypto_info_aes_gcm_128 crypto = {
        .info = {
            .version = TLS_1_2_VERSION,
            .cipher_type = TLS_CIPHER_AES_GCM_128,
        },
    };

    for (size_t i = 0; i < sizeof(crypto.key); i++)
        crypto.key[i] = (uint8_t)(0x10 + i);
    for (size_t i = 0; i < sizeof(crypto.salt); i++)
        crypto.salt[i] = (uint8_t)(0x30 + i);
    for (size_t i = 0; i < sizeof(crypto.iv); i++)
        crypto.iv[i] = (uint8_t)(0x40 + i);

    if (setsockopt(fd, IPPROTO_TCP, TCP_ULP, ulp, sizeof(ulp)) != 0)
        die("setsockopt(TCP_ULP=tls)");
    if (setsockopt(fd, SOL_TLS, TLS_TX, &crypto, sizeof(crypto)) != 0)
        die("setsockopt(TLS_TX)");
    if (setsockopt(fd, SOL_TLS, TLS_RX, &crypto, sizeof(crypto)) != 0)
        die("setsockopt(TLS_RX)");
}

static int tcp_state(int fd)
{
    struct tcp_info info = {0};
    socklen_t len = sizeof(info);
    if (getsockopt(fd, IPPROTO_TCP, TCP_INFO, &info, &len) != 0)
        die("getsockopt(TCP_INFO)");
    return info.tcpi_state;
}

static void destroy_with_sock_diag(int fd)
{
    struct sockaddr_in local;
    struct sockaddr_in remote;
    socklen_t address_len = sizeof(local);
    uint64_t cookie;
    socklen_t cookie_len = sizeof(cookie);
    if (getsockname(fd, (struct sockaddr *)&local, &address_len) != 0)
        die("getsockname(client)");
    address_len = sizeof(remote);
    if (getpeername(fd, (struct sockaddr *)&remote, &address_len) != 0)
        die("getpeername(client)");
    if (getsockopt(fd, SOL_SOCKET, SO_COOKIE, &cookie, &cookie_len) != 0)
        die("getsockopt(SO_COOKIE)");

    int diag = socket(AF_NETLINK, SOCK_RAW | SOCK_CLOEXEC, NETLINK_SOCK_DIAG);
    if (diag < 0)
        die("socket(NETLINK_SOCK_DIAG)");
    struct sockaddr_nl local_nl = {.nl_family = AF_NETLINK};
    if (bind(diag, (struct sockaddr *)&local_nl, sizeof(local_nl)) != 0)
        die("bind(NETLINK_SOCK_DIAG)");

    struct {
        struct nlmsghdr header;
        struct inet_diag_req_v2 request;
    } message = {
        .header = {
            .nlmsg_len = NLMSG_LENGTH(sizeof(struct inet_diag_req_v2)),
            .nlmsg_type = SOCK_DESTROY,
            .nlmsg_flags = NLM_F_REQUEST | NLM_F_ACK,
            .nlmsg_seq = 1,
        },
        .request = {
            .sdiag_family = AF_INET,
            .sdiag_protocol = IPPROTO_TCP,
            .idiag_states = UINT32_MAX,
        },
    };
    message.request.id.idiag_sport = local.sin_port;
    message.request.id.idiag_dport = remote.sin_port;
    message.request.id.idiag_src[0] = local.sin_addr.s_addr;
    message.request.id.idiag_dst[0] = remote.sin_addr.s_addr;
    message.request.id.idiag_cookie[0] = (uint32_t)cookie;
    message.request.id.idiag_cookie[1] = (uint32_t)(cookie >> 32);

    struct sockaddr_nl kernel = {.nl_family = AF_NETLINK};
    struct iovec iov = {.iov_base = &message, .iov_len = message.header.nlmsg_len};
    struct msghdr send_message = {
        .msg_name = &kernel,
        .msg_namelen = sizeof(kernel),
        .msg_iov = &iov,
        .msg_iovlen = 1,
    };
    if (sendmsg(diag, &send_message, 0) < 0)
        die("sendmsg(SOCK_DESTROY)");

    uint8_t reply[4096];
    ssize_t reply_len = recv(diag, reply, sizeof(reply), 0);
    if (reply_len < 0)
        die("recv(SOCK_DESTROY ack)");
    struct nlmsghdr *header = (struct nlmsghdr *)reply;
    if (!NLMSG_OK(header, (unsigned int)reply_len) || header->nlmsg_type != NLMSG_ERROR) {
        fprintf(stderr, "unexpected SOCK_DESTROY reply type=%u len=%zd\n",
                header->nlmsg_type, reply_len);
        exit(2);
    }
    struct nlmsgerr *error = NLMSG_DATA(header);
    if (error->error != 0) {
        errno = -error->error;
        die("SOCK_DESTROY ack");
    }
    close(diag);
}

static void child_peer(int listener, int ready_write, int applied_read, int outcome_write)
{
    int fd = accept(listener, NULL, NULL);
    if (fd < 0)
        die("accept");
    close(listener);

    struct timeval timeout = {.tv_sec = 0, .tv_usec = 100000};
    if (setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &timeout, sizeof(timeout)) != 0)
        die("setsockopt(SO_RCVTIMEO)");

    uint8_t byte;
    for (;;) {
        ssize_t got = recv(fd, &byte, sizeof(byte), 0);
        if (got == 0)
            break;
        if (got < 0 && (errno == EAGAIN || errno == EWOULDBLOCK))
            continue;
        if (got < 0)
            die("recv(before FIN)");
    }

    uint8_t ready = 1;
    write_exact(ready_write, &ready, sizeof(ready));

    uint8_t applied;
    read_exact(applied_read, &applied, sizeof(applied));
    struct timespec settle = {.tv_sec = 0, .tv_nsec = 250000000};
    nanosleep(&settle, NULL);

    errno = 0;
    ssize_t got = recv(fd, &byte, sizeof(byte), 0);
    int recv_errno = errno;
    int peer_state = tcp_state(fd);
    int peer_error = 0;
    socklen_t peer_error_len = sizeof(peer_error);
    if (getsockopt(fd, SOL_SOCKET, SO_ERROR, &peer_error, &peer_error_len) != 0)
        die("getsockopt(peer SO_ERROR)");
    struct peer_outcome outcome = {
        .state_after_candidate = peer_state,
        .so_error_after_candidate = peer_error,
        .recv_result = (int)got,
        .recv_errno = got < 0 ? recv_errno : 0,
    };
    write_exact(outcome_write, &outcome, sizeof(outcome));
    close(fd);
    close(ready_write);
    close(applied_read);
    close(outcome_write);
    _exit(0);
}

static bool run(enum candidate candidate)
{
    int listener = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (listener < 0)
        die("socket(listener)");
    struct sockaddr_in address = {
        .sin_family = AF_INET,
        .sin_addr = {.s_addr = htonl(INADDR_LOOPBACK)},
        .sin_port = 0,
    };
    if (bind(listener, (struct sockaddr *)&address, sizeof(address)) != 0)
        die("bind");
    if (listen(listener, 1) != 0)
        die("listen");
    socklen_t address_len = sizeof(address);
    if (getsockname(listener, (struct sockaddr *)&address, &address_len) != 0)
        die("getsockname");

    int ready[2];
    int applied[2];
    int outcome[2];
    if (pipe2(ready, O_CLOEXEC) != 0 || pipe2(applied, O_CLOEXEC) != 0 ||
        pipe2(outcome, O_CLOEXEC) != 0)
        die("pipe2");

    pid_t child = fork();
    if (child < 0)
        die("fork");
    if (child == 0) {
        close(ready[0]);
        close(applied[1]);
        close(outcome[0]);
        child_peer(listener, ready[1], applied[0], outcome[1]);
    }

    close(ready[1]);
    close(applied[0]);
    close(outcome[1]);
    int fd = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (fd < 0)
        die("socket(client)");
    if (connect(fd, (struct sockaddr *)&address, sizeof(address)) != 0)
        die("connect");
    close(listener);
    arm_ktls(fd);

    if (shutdown(fd, SHUT_WR) != 0)
        die("shutdown(SHUT_WR)");
    uint8_t ready_byte;
    read_exact(ready[0], &ready_byte, sizeof(ready_byte));

    int state = tcp_state(fd);
    struct timespec state_pause = {.tv_sec = 0, .tv_nsec = 10000000};
    for (unsigned attempt = 0; state != TCP_FIN_WAIT2 && attempt < 200; attempt++) {
        nanosleep(&state_pause, NULL);
        state = tcp_state(fd);
    }
    if (state != TCP_FIN_WAIT2) {
        fprintf(stderr, "socket did not reach TCP_FIN_WAIT2: state=%d\n", state);
        exit(2);
    }
    struct linger abortive = {.l_onoff = 1, .l_linger = 0};
    if (setsockopt(fd, SOL_SOCKET, SO_LINGER, &abortive, sizeof(abortive)) != 0)
        die("setsockopt(SO_LINGER)");

    if (candidate == CANDIDATE_LINGER2_NEGATIVE) {
        int linger2 = -1;
        if (setsockopt(fd, IPPROTO_TCP, TCP_LINGER2, &linger2, sizeof(linger2)) != 0)
            die("setsockopt(TCP_LINGER2)");
    } else if (candidate == CANDIDATE_TCP_REPAIR) {
        int enabled = 1;
        if (setsockopt(fd, IPPROTO_TCP, TCP_REPAIR, &enabled, sizeof(enabled)) != 0)
            die("setsockopt(TCP_REPAIR=1)");
    } else if (candidate == CANDIDATE_SOCK_DESTROY) {
        destroy_with_sock_diag(fd);
    }

    close(fd);
    uint8_t applied_byte = 1;
    write_exact(applied[1], &applied_byte, sizeof(applied_byte));
    struct peer_outcome peer;
    read_exact(outcome[0], &peer, sizeof(peer));
    close(ready[0]);
    close(applied[1]);
    close(outcome[0]);
    int status;
    if (waitpid(child, &status, 0) < 0)
        die("waitpid");
    if (!WIFEXITED(status) || WEXITSTATUS(status) != 0) {
        fprintf(stderr, "peer exited abnormally: status=%d\n", status);
        exit(2);
    }

    printf("candidate=%s preclose_tcp_state=%d peer_state=%d peer_so_error=%d post_fin_recv=%d post_fin_recv_errno=%d\n",
           candidate == CANDIDATE_LINGER_ZERO ? "SO_LINGER0" :
           candidate == CANDIDATE_LINGER2_NEGATIVE ? "TCP_LINGER2_NEGATIVE" :
           candidate == CANDIDATE_TCP_REPAIR ? "TCP_REPAIR" : "SOCK_DESTROY",
           state,
           peer.state_after_candidate,
           peer.so_error_after_candidate,
           peer.recv_result,
           peer.recv_errno);
    return peer.state_after_candidate == TCP_CLOSE &&
           (peer.so_error_after_candidate == EPIPE ||
            peer.so_error_after_candidate == ECONNRESET ||
            peer.so_error_after_candidate == ECONNABORTED);
}

int main(void)
{
    (void)run(CANDIDATE_LINGER_ZERO);
    (void)run(CANDIDATE_LINGER2_NEGATIVE);
    (void)run(CANDIDATE_TCP_REPAIR);
    (void)run(CANDIDATE_SOCK_DESTROY);
    return 0;
}
