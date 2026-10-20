/* SPDX-License-Identifier: GPL-2.0 WITH Linux-syscall-note */
/* ev-pair spike: the PROPOSED /dev/ovd_fwd control shape, reduced to what V-EV-PAIR needs
 * (ATTACH, ABORT, VERSION, 32-byte end record via read()/poll(), release aborts every pair). Not the DESIGN contract. */
#ifndef OVD_FWD_UAPI_H
#define OVD_FWD_UAPI_H
#include <linux/types.h>
#include <linux/ioctl.h>

#define OVD_ABI 1
#define OVD_F_PERMANENT 1u

struct ovd_version { __u32 abi; __u32 flags; };

/* early_flags */
#define OVD_EARLY_END_A   1u   /* an end (TCP FIN / vsock SEND_SHUTDOWN) was already queued on half A at attach */
#define OVD_EARLY_END_B   2u
#define OVD_EARLY_ABORT_A 4u   /* a vsock half had already received 0x3/RST at attach: classified abort */
#define OVD_EARLY_ABORT_B 8u

struct ovd_attach {
	__s32 fd_a;
	__s32 fd_b;
	__u32 flags;      /* must be 0 */
	__u32 early_flags;/* out */
	__u64 id;         /* out */
	__u64 early_bytes_a; /* out: bytes already queued on half A when the attach took the socket lock */
	__u64 early_bytes_b; /* out */
};

#define OVD_VERSION _IOR('F', 1, struct ovd_version)
#define OVD_ATTACH  _IOWR('F', 2, struct ovd_attach)
#define OVD_ABORT   _IOW('F', 3, __u64)

#define OVD_OUT_COMPLETED 1
#define OVD_OUT_ABORTED   2
#define OVD_CAUSE_NONE         0
#define OVD_CAUSE_SOCKET_ERROR 1  /* side, err */
#define OVD_CAUSE_PEER_ABORT   2  /* side */
#define OVD_CAUSE_DETACHED     3  /* owner ABORT */
#define OVD_CAUSE_UNCLASSIFIED 4  /* side */
#define OVD_CAUSE_RELEASED     5  /* context release: no record is queued */

struct ovd_end {          /* 32 bytes */
	__u64 id;
	__u8  outcome;
	__u8  cause;
	__u8  side;       /* 0 = A, 1 = B */
	__u8  pad;
	__s32 err;
	__u64 bytes_a2b;
	__u64 bytes_b2a;
};
#endif
