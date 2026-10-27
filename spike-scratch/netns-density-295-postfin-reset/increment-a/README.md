# Post-FIN kTLS reset candidate

This bounded native probe asks one question: after a local kTLS socket has sent
FIN and reached `TCP_FIN_WAIT2` while its peer keeps the reverse direction open,
which final-close mechanism makes that peer observe reset after FIN?

It compares the already-falsified `SO_LINGER{1,0}` and
`TCP_LINGER2=-1` shapes with `TCP_REPAIR=1` and an exact-cookie
`NETLINK_SOCK_DIAG/SOCK_DESTROY` request immediately before the final close.
The peer reads raw TCP after FIN, then records `TCP_INFO.tcpi_state` and
`SO_ERROR`. This distinguishes the unchanged STEP13-01 cell's post-EOF read
oracle from the endpoint's actual reset state. The probe creates only loopback
sockets and does not touch modules, cgroups, management networking, or
production files.

Run on the qualified host as root because `TCP_REPAIR` requires
`CAP_NET_ADMIN`:

```sh
bash run.sh
```

The unchanged production-path cell can be surrounded with a loopback FIN/RST
capture without editing its body:

```sh
bash capture-production-rst.sh
```
