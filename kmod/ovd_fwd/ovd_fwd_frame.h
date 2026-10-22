/* SPDX-License-Identifier: GPL-2.0 WITH Linux-syscall-note */
#ifndef OVD_FWD_FRAME_H
#define OVD_FWD_FRAME_H

#include <linux/types.h>
#include <linux/ovd_fwd.h>

enum ovd_fwd_frame_outcome {
	OVD_FWD_FRAME_NEED_MORE = 0,
	OVD_FWD_FRAME_COMPLETE = 1,
	OVD_FWD_FRAME_OVERSIZE = 2,
	OVD_FWD_FRAME_INVALID = 3,
};

enum ovd_fwd_frame_outcome ovd_fwd_frame_parse(const __u8 *buf,
					       __u32 buf_len,
					       __u64 avail,
					       __u32 *need,
					       __u32 *len);

#endif /* OVD_FWD_FRAME_H */
