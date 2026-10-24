// SPDX-License-Identifier: GPL-2.0
// Fast Open option candidate captured before TCP ACK validation.
#include "vmlinux.h"
#include <bpf/bpf_core_read.h>
#include <bpf/bpf_endian.h>
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_tracing.h>

#define TCPOPT_EXP 254
#define TCP_OPTION_LEN 18
#define TCP_OPTION_MAGIC 0x4f56
#define TCP_SYN 0x02
#define TCP_ACK 0x10

#define CHILD_PENDING 1
#define CHILD_READY 2
#define CHILD_REJECTED 3
#define CHILD_CANDIDATE_READY 4
#define CHILD_CANDIDATE_REJECTED 5

struct correlation_meta {
    __u64 application_cookie;
    __u32 destination_ip;
    __u16 destination_port;
    __u16 pad;
};

struct child_state {
    __u32 state;
    __u32 pad;
    struct correlation_meta meta;
};

struct correlation_option {
    __u8 kind;
    __u8 len;
    __be16 magic;
    __be64 application_cookie;
    __u32 destination_ip;
    __be16 destination_port;
} __attribute__((packed));

_Static_assert(sizeof(struct correlation_meta) == 16, "correlation metadata ABI");
_Static_assert(sizeof(struct child_state) == 24, "child correlation ABI");
_Static_assert(sizeof(struct correlation_option) == TCP_OPTION_LEN, "TCP option ABI");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 8194);
    __type(key, __u64);
    __type(value, struct child_state);
    __uint(pinning, LIBBPF_PIN_BY_NAME);
} CHILD_CORRELATION SEC(".maps");

static __always_inline bool is_non_syn_ack(__u8 flags)
{
    return (flags & (TCP_SYN | TCP_ACK)) == TCP_ACK;
}

static __always_inline bool load_correlation_option(
    struct sk_buff *skb, struct correlation_meta *meta)
{
    const unsigned char *data = BPF_CORE_READ(skb, data);
    __u8 tcp[20];

    /* tcp_v4_rcv makes the complete doff-sized header linear before
     * tcp_check_req. CO-RE resolves skb::data; this helper reads only kernel
     * memory containing the TCP header and options.
     */
    if (!data || bpf_probe_read_kernel(tcp, sizeof(tcp), data))
        return false;
    if (!is_non_syn_ack(tcp[13]))
        return false;

    __u32 header_len = ((__u32)tcp[12] >> 4) * 4;
    if (header_len < sizeof(tcp) || header_len > 60)
        return false;

#pragma unroll
    for (int offset = 0; offset < 23; offset++) {
        struct correlation_option option;

        if (20 + offset + TCP_OPTION_LEN > header_len)
            continue;
        if (bpf_probe_read_kernel(&option, sizeof(option), data + 20 + offset))
            return false;
        if (option.kind != TCPOPT_EXP || option.len != TCP_OPTION_LEN ||
            bpf_ntohs(option.magic) != TCP_OPTION_MAGIC)
            continue;

        meta->application_cookie = bpf_be64_to_cpu(option.application_cookie);
        meta->destination_ip = option.destination_ip;
        meta->destination_port = bpf_ntohs(option.destination_port);
        meta->pad = 0;
        return true;
    }
    return false;
}

SEC("fentry/tcp_check_req")
int BPF_PROG(tcp_correlation_candidate, struct sock *sk, struct sk_buff *skb,
             struct request_sock *req, bool fastopen, bool *req_stolen,
             enum skb_drop_reason *drop_reason)
{
    __u64 child;
    struct child_state *current;
    struct child_state next = {};
    __u32 current_state;

    (void)req;
    (void)req_stolen;
    (void)drop_reason;

    if (!fastopen)
        return 0;

    child = bpf_get_socket_cookie(sk);
    current = bpf_map_lookup_elem(&CHILD_CORRELATION, &child);
    if (!current)
        return 0;
    current_state = current->state;
    if (current_state != CHILD_PENDING &&
        current_state != CHILD_CANDIDATE_READY &&
        current_state != CHILD_CANDIDATE_REJECTED)
        return 0;

    if (load_correlation_option(skb, &next.meta))
        next.state = CHILD_CANDIDATE_READY;
    else
        next.state = CHILD_CANDIDATE_REJECTED;

    /* Replacement is serialized by the child socket lock. BPF_EXIST keeps
     * concurrent consume or sock_release deletion final. */
    bpf_map_update_elem(&CHILD_CORRELATION, &child, &next, BPF_EXIST);
    return 0;
}

char LICENSE[] SEC("license") = "GPL";
