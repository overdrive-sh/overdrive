# Step 11-06 immediate UDP release investigation — Increment C

This bounded review probe uses the existing increment-B host and runner with
one ordinary guest workload variation. It drives the production static
`overdrive-init` as PID 1 through GuestKit and CloudHypervisorVmm, using the
default workload image. All routing, module and BPF effects are inside the
disposable guest on the existing leased Metal runner. No production or
permanent test source is modified.

Hypothesis: following F3 remediation, an application that sends and closes
before `KernelGuestOwnerIo::slot_owner` obtains its diagnostic source port can
leave a kernel slot claimed without a published guest-owner slot owner.
`SocketReleased` and the audit currently select only published owners. The
source path to assess is `sendmsg4 -> alloc_slot -> sock_release`, then
`poll_once -> SlotReady -> start_datagram -> slot_owner`, and the
`SocketReleased`/`audit_slots` cleanup paths.

Prediction: 1,100 ordinary send/drop operations can fill the 1,024-slot pool;
after all application sockets close and three 1-second audit periods pass, a
fresh ordinary held socket is still refused with EPERM. The host owner remains
alive. This would violate eventual slot release and below-capacity admission.

Falsification: the post-audit fresh socket sends and receives the host's exact
echo. No requirement is placed on fire-and-forget replies or on temporary
EPERM while cleanup is pending.

Run `bash spike-scratch/netns-density-295-review-11-06/increment-c/run.sh default-kit`.
The runner retains commands, full output, guest console and exit status in
`evidence/reruns/`. Its expected success is `POST_AUDIT_RESULT=ECHOED`; the
underlying host checks the production guest's `EXIT 0`.
