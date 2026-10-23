// SPDX-License-Identifier: GPL-2.0
// Throwaway CO-RE proof. The production tree does not consume this object.
#include "vmlinux.h"
#include <bpf/bpf_core_read.h>
#include <bpf/bpf_endian.h>
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_tracing.h>

#define TCPOPT_EXP 254
#define OPT_LEN 18
#define MAGIC 0x4f56
#define TCP_SYN 0x02
#define TCP_ACK 0x10

#define PENDING 1
#define CANDIDATE_READY 4
#define CANDIDATE_REJECT 5

#define C_TFO_CANDIDATE_READY 12
#define C_TFO_CANDIDATE_REJECT 13
#define C_TERMINAL_NOOP 14

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

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 8194);
    __type(key, __u64);
    __type(value, struct child_state);
    __uint(pinning, LIBBPF_PIN_BY_NAME);
} CHILD SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_PERCPU_ARRAY);
    __uint(max_entries, 16);
    __type(key, __u32);
    __type(value, __u64);
    __uint(pinning, LIBBPF_PIN_BY_NAME);
} COUNTERS SEC(".maps");

static __always_inline void bump(__u32 index)
{
    __u64 *count = bpf_map_lookup_elem(&COUNTERS, &index);

    if (count)
        *count += 1;
}

static __always_inline bool is_non_syn_ack(__u8 flags)
{
    return (flags & (TCP_SYN | TCP_ACK)) == TCP_ACK;
}

static __always_inline bool load_option(struct sk_buff *skb,
                                        struct correlation_meta *meta)
{
    unsigned char *data = BPF_CORE_READ(skb, data);
    __u8 tcp[20];

    /* tcp_v4_rcv has made the complete doff-sized TCP header linear before
     * tcp_check_req. CO-RE resolves the real sk_buff::data field; the helper
     * performs the kernel-memory copy without an skb ABI offset clone.
     */
    if (!data || bpf_probe_read_kernel(tcp, sizeof(tcp), data))
        return false;
    if (!is_non_syn_ack(tcp[13]))
        return false;

    __u32 header_len = (__u32)(tcp[12] >> 4) * 4;
    if (header_len < sizeof(tcp) || header_len > 60)
        return false;

#pragma unroll
    for (int offset = 0; offset < 23; offset++) {
        struct correlation_option option;

        if (20 + offset + OPT_LEN > header_len)
            continue;
        if (bpf_probe_read_kernel(&option, sizeof(option), data + 20 + offset))
            return false;
        if (option.kind != TCPOPT_EXP || option.len != OPT_LEN ||
            bpf_ntohs(option.magic) != MAGIC)
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
int BPF_PROG(trace_fastopen_candidate, struct sock *sk, struct sk_buff *skb,
             struct request_sock *req, bool fastopen, bool *req_stolen,
             enum skb_drop_reason *drop_reason)
{
    __u64 child;
    struct child_state *current;
    struct child_state next = {};

    if (!fastopen)
        return 0;
    child = bpf_get_socket_cookie(sk);
    current = bpf_map_lookup_elem(&CHILD, &child);
    if (!current)
        return 0;
    if (current->state != PENDING && current->state != CANDIDATE_READY &&
        current->state != CANDIDATE_REJECT) {
        bump(C_TERMINAL_NOOP);
        return 0;
    }

    if (load_option(skb, &next.meta)) {
        next.state = CANDIDATE_READY;
        bump(C_TFO_CANDIDATE_READY);
    } else {
        next.state = CANDIDATE_REJECT;
        bump(C_TFO_CANDIDATE_REJECT);
    }
    bpf_map_update_elem(&CHILD, &child, &next, BPF_EXIST);
    return 0;
}

char LICENSE[] SEC("license") = "GPL";
