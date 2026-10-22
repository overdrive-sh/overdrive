/* SPDX-License-Identifier: GPL-2.0 WITH Linux-syscall-note */
#ifndef _UAPI_LINUX_OVD_FWD_H
#define _UAPI_LINUX_OVD_FWD_H

#include <linux/ioctl.h>
#include <linux/types.h>

#define OVD_FWD_ABI_VERSION 1U

#define OVD_FWD_FRAME_MAGIC        0x4f564431U
#define OVD_FWD_FRAME_HDR_LEN      8U
#define OVD_FWD_FRAME_MAX_PAYLOAD  59000U
#define OVD_FWD_FRAME_MAX          59008U

struct ovd_fwd_version {
	__u32 abi_version;
};

struct ovd_fwd_attach;
struct ovd_fwd_watch;
struct ovd_fwd_inventory;
struct ovd_fwd_pair_state;

#define OVD_FWD_VERSION     _IOR(0xB9, 0x00, struct ovd_fwd_version)
#define OVD_FWD_ATTACH      _IOWR(0xB9, 0x01, struct ovd_fwd_attach)
#define OVD_FWD_WATCH       _IOWR(0xB9, 0x02, struct ovd_fwd_watch)
#define OVD_FWD_ABORT       _IOW(0xB9, 0x03, __u64)
#define OVD_FWD_ABORT_ALL   _IO(0xB9, 0x04)
#define OVD_FWD_INVENTORY   _IOR(0xB9, 0x05, struct ovd_fwd_inventory)
#define OVD_FWD_PAIR_STATE  _IOWR(0xB9, 0x06, struct ovd_fwd_pair_state)

#endif /* _UAPI_LINUX_OVD_FWD_H */
