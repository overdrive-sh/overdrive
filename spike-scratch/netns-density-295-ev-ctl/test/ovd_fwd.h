/* SPDX-License-Identifier: GPL-2.0 WITH Linux-syscall-note */
/*
 * ev-ctl spike: the PROPOSED EVMOD-CTL control ABI (feature-delta "Control interface") as the spike module implements it.
 * Spike-only deviations are marked SPIKE.  Not a design contract.
 */
#ifndef OVD_FWD_UAPI_H
#define OVD_FWD_UAPI_H
#include <linux/types.h>
#include <linux/ioctl.h>

#define OVD_FWD_ABI_VERSION 1
#define OVD_FWD_FLAG_PERMANENT 1u

#define OVD_FWD_MODE_STREAM 0u
#define OVD_FWD_MODE_DGRAM_UDP_A 1u /* SPIKE: validated, not forwarded (V-EV-DGRAM's) -> EOPNOTSUPP */
#define OVD_FWD_MODE_DGRAM_UDP_B 2u

struct ovd_fwd_version { __u32 abi_version; __u32 flags; };

struct ovd_fwd_attach {
	__s32 fd_a;
	__s32 fd_b;
	__u32 mode;
	__u32 flags;      /* 0 */
	__u64 id;         /* out */
	__u8  bad_side;   /* out on error: 0 = half A, 1 = half B */
	__u8  pad[7];
};

struct ovd_fwd_watch {
	__s32 fd;
	__u32 flags;      /* 0 */
	__u64 id;         /* out */
	__u8  bad_side;   /* out on error (always 0) */
	__u8  pad[7];
};

struct ovd_fwd_id { __u64 id; };
struct ovd_fwd_count { __u32 n; __u32 pad; };

struct ovd_fwd_inventory {
	__u32 live_pairs, live_watches, queued_ends, rsv;
	__u64 attached, watched, completed, half_closed;
	__u64 aborted_peer, aborted_sockerr, aborted_frame, aborted_detached;
	__u64 unclassified_eof, dgram_oversize;
};

struct ovd_fwd_pair_state {
	__u64 id;                /* in */
	__u8 source_end[2];      /* out per direction (0 a->b, 1 b->a): 0 none, 1 clean, 2 abort */
	__u8 target_shut[2];
	__u8 pad[4];
	__u64 moved[2];
};

/* 32-byte end record, native byte order */
struct ovd_fwd_end {
	__u64 id;
	__u8  kind;      /* 1 pair completed, 2 pair aborted, 3 watch guest half-close, 4 watch aborted */
	__u8  cause;     /* kinds 2, 4: 1 peer abort, 2 socket error, 3 frame invalid, 4 detached */
	__u8  side;
	__u8  rsv0;
	__u16 err;
	__u16 rsv1;
	__u64 moved_ab;
	__u64 moved_ba;
};

#define OVD_FWD_IOC 'F'
#define OVD_FWD_VERSION    _IOR(OVD_FWD_IOC, 1, struct ovd_fwd_version)
#define OVD_FWD_ATTACH     _IOWR(OVD_FWD_IOC, 2, struct ovd_fwd_attach)
#define OVD_FWD_WATCH      _IOWR(OVD_FWD_IOC, 3, struct ovd_fwd_watch)
#define OVD_FWD_ABORT      _IOW(OVD_FWD_IOC, 4, struct ovd_fwd_id)
#define OVD_FWD_ABORT_ALL  _IOR(OVD_FWD_IOC, 5, struct ovd_fwd_count)
#define OVD_FWD_INVENTORY  _IOR(OVD_FWD_IOC, 6, struct ovd_fwd_inventory)
#define OVD_FWD_PAIR_STATE _IOWR(OVD_FWD_IOC, 7, struct ovd_fwd_pair_state)
#endif
