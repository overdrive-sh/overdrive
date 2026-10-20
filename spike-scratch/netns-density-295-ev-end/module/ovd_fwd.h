/* SPDX-License-Identifier: GPL-2.0 WITH Linux-syscall-note */
/* ev-end spike: the PROPOSED EVMOD-CTL control ABI (stream mode and watch only). */
#ifndef OVD_FWD_UAPI_H
#define OVD_FWD_UAPI_H
#include <linux/types.h>
#include <linux/ioctl.h>

#define OVD_FWD_ABI 1
#define OVD_FWD_FLAG_PERMANENT 1u

struct ovd_fwd_version { __u32 abi_version; __u32 flags; };
struct ovd_fwd_attach { __s32 fd_a; __s32 fd_b; __u32 mode; __u32 flags; __u64 id; __u8 bad_side; __u8 pad[7]; };
struct ovd_fwd_watch { __s32 fd; __u32 pad; __u64 id; };
struct ovd_fwd_inventory {
	__u32 live_pairs, live_watches, queued_ends, pad;
	__u64 attached, completed, aborted, unclassified_eof, dgram_oversize;
	__u64 ab_peer, ab_socket, ab_frame, ab_detached;   /* aborted, by cause */
};
struct ovd_fwd_dir { __u8 source_end; __u8 target_shut; __u8 pad[6]; __u64 moved; };   /* source_end: 0 none 1 clean 2 abort */
struct ovd_fwd_pair_state { __u64 id; struct ovd_fwd_dir dir[2]; };

/* end record, 32 bytes, native byte order */
struct ovd_fwd_end {
	__u64 id;
	__u8 kind;     /* 1 pair completed, 2 pair aborted, 3 watch guest half-close, 4 watch aborted */
	__u8 cause;    /* kinds 2 and 4: 1 peer abort, 2 socket error, 3 frame invalid, 4 detached */
	__u8 side;     /* causes 1-3: 0 half A, 1 half B */
	__u8 rsvd0;
	__u16 err;     /* cause 2: errno */
	__u16 rsvd1;
	__u64 moved_ab;
	__u64 moved_ba;
};

#define OVD_FWD_VERSION    _IOR('F', 1, struct ovd_fwd_version)
#define OVD_FWD_ATTACH     _IOWR('F', 2, struct ovd_fwd_attach)
#define OVD_FWD_WATCH      _IOWR('F', 3, struct ovd_fwd_watch)
#define OVD_FWD_ABORT      _IOW('F', 4, __u64)
#define OVD_FWD_ABORT_ALL  _IOR('F', 5, __u32)
#define OVD_FWD_INVENTORY  _IOR('F', 6, struct ovd_fwd_inventory)
#define OVD_FWD_PAIR_STATE _IOWR('F', 7, struct ovd_fwd_pair_state)
#endif
