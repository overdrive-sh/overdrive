// Spike B3 stretch (GH #303, increment-f): an ordinary Go HTTP client, standard
// library only. It knows nothing about TLS or the mesh; it does one plain HTTP
// GET. The Go runtime brings its own netpoller (non-blocking sockets + epoll,
// edge-triggered), several OS threads (clone), futexes and timers.
//
//	igkmf-go <url>
package main

import (
	"fmt"
	"io"
	"net/http"
	"os"
	"runtime"
	"time"
)

func main() {
	fmt.Printf("IGKM-F-GO: %s %s/%s GOMAXPROCS=%d NumCPU=%d args=%q\n",
		runtime.Version(), runtime.GOOS, runtime.GOARCH,
		runtime.GOMAXPROCS(0), runtime.NumCPU(), os.Args)
	if len(os.Args) < 2 {
		os.Exit(2)
	}
	url := os.Args[1]
	c := &http.Client{Timeout: 20 * time.Second}
	t0 := time.Now()
	resp, err := c.Get(url)
	if err != nil {
		fmt.Printf("IGKM-F-GO: GET %s failed after %v: %v\n", url, time.Since(t0), err)
		os.Exit(1)
	}
	body, err := io.ReadAll(resp.Body)
	resp.Body.Close()
	fmt.Printf("IGKM-F-GO: GET %s -> %q proto=%s content-length=%d, read %d body bytes (err=%v) after %v\n",
		url, resp.Status, resp.Proto, resp.ContentLength, len(body), err, time.Since(t0))
	os.Stdout.Write(body)
	fmt.Printf("IGKM-F-GO: done, goroutines=%d\n", runtime.NumGoroutine())
}
