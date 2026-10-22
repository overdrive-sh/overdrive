/* SPDX-License-Identifier: GPL-2.0 WITH Linux-syscall-note */
#ifndef _UAPI_LINUX_OVD_FWD_H
#define _UAPI_LINUX_OVD_FWD_H

#include <linux/ioctl.h>
#include <linux/types.h>

#define OVD_FWD_ABI_VERSION 1U

#define OVD_FWD_MODE_STREAM 0U
#define OVD_FWD_MODE_DGRAM_UDP_A 1U
#define OVD_FWD_MODE_DGRAM_UDP_B 2U

#define OVD_FWD_FRAME_MAGIC 0x4f564431U
#define OVD_FWD_FRAME_HDR_LEN 8U
#define OVD_FWD_FRAME_MAX_PAYLOAD 59000U
#define OVD_FWD_FRAME_MAX 59008U

struct ovd_fwd_version {
	__u32 abi_version;
};

struct ovd_fwd_attach {
	__s32 fd_a;
	__s32 fd_b;
	__u32 mode;
	__u32 flags;
	__u64 id;
	__u8 bad_side;
	__u8 reserved0;
	__u16 end_errno;
	__u32 reserved1;
};

struct ovd_fwd_watch {
	__s32 fd;
	__u32 flags;
	__u64 id;
	__u8 bad_side;
	__u8 reserved0;
	__u16 end_errno;
	__u32 reserved1;
};

struct ovd_fwd_inventory {
	__u32 live_pairs;
	__u32 live_watches;
	__u32 queued_ends;
	__u32 reserved;
	__u64 attached;
	__u64 completed;
	__u64 aborted_peer;
	__u64 aborted_socket;
	__u64 aborted_frame;
	__u64 aborted_detached;
	__u64 unclassified_eof;
	__u64 dgram_oversize;
};

struct ovd_fwd_pair_state {
	__u64 id;
	__u8 source_end[2];
	__u8 target_shut[2];
	__u8 reserved[4];
	__u64 moved[2];
};

struct ovd_fwd_end {
	__u64 id;
	__u8 kind;
	__u8 cause;
	__u8 side;
	__u8 reserved0;
	__u16 errno;
	__u16 reserved1;
	__u64 moved_ab;
	__u64 moved_ba;
};

#define OVD_FWD_VERSION     _IOR(0xB9, 0x00, struct ovd_fwd_version)
#define OVD_FWD_ATTACH      _IOWR(0xB9, 0x01, struct ovd_fwd_attach)
#define OVD_FWD_WATCH       _IOWR(0xB9, 0x02, struct ovd_fwd_watch)
#define OVD_FWD_ABORT       _IOW(0xB9, 0x03, __u64)
#define OVD_FWD_ABORT_ALL   _IO(0xB9, 0x04)
#define OVD_FWD_INVENTORY   _IOR(0xB9, 0x05, struct ovd_fwd_inventory)
#define OVD_FWD_PAIR_STATE  _IOWR(0xB9, 0x06, struct ovd_fwd_pair_state)

#endif /* _UAPI_LINUX_OVD_FWD_H */
