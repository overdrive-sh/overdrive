# DNS/53 owner and sequence diagnosis

This bounded GuestKit diagnostic runs the existing DNS fallback case first in
a fresh control-plane guest, then in a second fresh guest after the exact two
preceding adoption cases. Diagnostic-only output in the existing DNS body
retains every UDP/53 kernel row and its PID/fd/inode/socket flag/current-netns/
command owner before and after the body creates its private stand-in holder.

The harness fails if either adoption case skips or fails, if the fresh and
prefix DNS outcomes differ, or if any UDP/53 owner exists before the DNS body
creates its own holder. It adds no selectable group, CI lane, scenario,
executor, product API, or physical-host network effect.

## Result

Durable Metal `a1a5715691ae238e0a7032a7` exited zero. Both fresh and
two-adoption-prefix runs proved:

- no UDP/53 socket or owner existed before the DNS body;
- only loopback `127.0.0.1/8` existed and no local route covered
  `100.95.0.1/32`;
- the body itself created PID-local fd 9/inode 2103 or 2158 at
  `0.0.0.0:53`, in root netns `4026531833`, with `SO_REUSEADDR=0`,
  `SO_REUSEPORT=0`, and `FD_CLOEXEC=1`; and
- both adoption cases passed and left no DNS descriptor.

Both DNS executions then failed on the fixture itself. The empty gateway route
surfaced `EADDRNOTAVAIL`; with the qualified gateway present, Linux's overlap
predicate also rejects the exact fallback because the body's own wildcard
holder has reuse disabled. The failure is intrinsic fixture input plus a
missing qualified-node `/32` route, not adoption lifecycle leakage.

Raw receipt: `evidence/metal-a1a5715691ae238e0a7032a7.log`, SHA-256
`d82c5a2ad66c928e9b8edc59ec3082073c18d9a975fe49f3db720a5e0394e209`.
