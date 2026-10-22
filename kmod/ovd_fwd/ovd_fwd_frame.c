// SPDX-License-Identifier: GPL-2.0
#include "ovd_fwd_frame.h"

enum ovd_fwd_frame_outcome ovd_fwd_frame_parse(const __u8 *buf,
					       __u32 buf_len,
					       __u64 avail,
					       __u32 *need,
					       __u32 *len)
{
	__u32 magic;
	__u32 payload_len;
	__u64 frame_len;

	*need = 0;
	*len = 0;
	if (avail < buf_len)
		return OVD_FWD_FRAME_INVALID;
	if (buf_len < OVD_FWD_FRAME_HDR_LEN) {
		*need = OVD_FWD_FRAME_HDR_LEN;
		return OVD_FWD_FRAME_NEED_MORE;
	}
	if (!buf)
		return OVD_FWD_FRAME_INVALID;

	magic = ((__u32)buf[0] << 24) | ((__u32)buf[1] << 16) |
		((__u32)buf[2] << 8) | (__u32)buf[3];
	if (magic != OVD_FWD_FRAME_MAGIC)
		return OVD_FWD_FRAME_INVALID;

	payload_len = ((__u32)buf[4] << 24) | ((__u32)buf[5] << 16) |
		((__u32)buf[6] << 8) | (__u32)buf[7];
	frame_len = (__u64)OVD_FWD_FRAME_HDR_LEN + (__u64)payload_len;
	if (frame_len > OVD_FWD_FRAME_MAX) {
		*len = payload_len;
		return OVD_FWD_FRAME_OVERSIZE;
	}
	*need = (__u32)frame_len;
	*len = payload_len;
	if (avail >= frame_len)
		return OVD_FWD_FRAME_COMPLETE;
	return OVD_FWD_FRAME_NEED_MORE;
}
