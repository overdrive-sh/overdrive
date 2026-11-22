import os, signal, sys
path, marker = sys.argv[1], sys.argv[2]
fd = os.open(path, os.O_RDWR | os.O_EXCL)
open(marker, "w").close()
signal.pause()
