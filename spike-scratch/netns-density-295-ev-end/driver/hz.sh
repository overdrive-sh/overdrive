cd /var/tmp/ev-end-work/src/driver; R="timeout 600 python3 run.py"
$R hz1 --hazard-host 1 --hazard-guest 1 --wait-ms 700 --timeout 500 -- g-rst-both-dirs:10 r-rst-both-dirs:10 host-abort:6 guest-abort:6 g-close-unread:6 dial-watch-reset:10 clean-g-first:10 vmm-kill:2 | grep -E "RESULT|FAIL|QEMU exit"
$R hz2 --hazard-host 2 --hazard-guest 2 --wait-ms 700 --timeout 500 -- clean-g-first:100 clean-fullend-0x2-0x3-hostlag:100 clean-fullend-0x2-0x3-guestlag:100 guest-abort-0x2then0x3-hostdir-open:100 guest-abort-direct0x3-hostdir-closed:100 host-abort-0x2then0x3-guestdir-open:100 g-rst-both-dirs:100 | grep -E "RESULT|FAIL|QEMU exit"
$R hz16 --hazard-host 16 --hazard-guest 16 --wait-ms 700 --timeout 500 -- g-rst-hg-only:20 r-rst-gh-only:20 g-rst-both-dirs:20 dial-watch-halfclose-then-reset:10 | grep -E "RESULT|FAIL|QEMU exit"
$R hz4 --hazard-host 4 --wait-ms 700 --timeout 500 -- dial-watch-reset:300 dial-watch-halfclose:300 dial-watch-reset-poll:300 dial-watch-halfclose-recordcheck-at-end:300 | grep -E "RESULT|FAIL|QEMU exit"
$R hz8 --hazard-host 8 --wait-ms 700 --timeout 500 -- dial-watch-reset:300 dial-watch-halfclose:300 dial-watch-reset-poll:300 dial-watch-halfclose-recordcheck-at-end:300 | grep -E "RESULT|FAIL|QEMU exit"
echo "stray ev-end processes:"; pgrep -af "ev-end-work" | grep -v pgrep || echo none
