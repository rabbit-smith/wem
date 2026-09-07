// Command wem-go-cgo is the reference Go binding for the WEM encoder
// kernel: a thin cgo shell over the C ABI core surface (include/wem.h).
//
// It encodes a signed-16 PCM WAV file one-shot into a WEM container
// through wem_encode_pcm16_interleaved and writes the container bytes.
// The kernel does all the work; this program only moves bytes.
//
// Usage:
//
//	go run . -wav tests/fixtures/input.wav -out out.wem
//
// Build steps (three lines): examples/go-cgo/README.md
package main

/*
#cgo CFLAGS: -I${SRCDIR}/../../include
#cgo LDFLAGS: -L${SRCDIR}/../../crates/target/release -lwem_capi -Wl,-rpath,${SRCDIR}/../../crates/target/release

#include <wem.h>
#include <stdlib.h>
#include <string.h>

// A growable byte sink; the C write callback appends the encoded bytes.
// The kernel writes through this callback in bounded blocks, so the
// whole container arrives across one or more calls (no size limit).
typedef struct {
    uint8_t *p;
    size_t len;
    size_t cap;
} wem_buf_t;

WemError wem_buf_write(const uint8_t *data, size_t len, void *user_data) {
    wem_buf_t *b = (wem_buf_t *)user_data;
    if (b->len + len > b->cap) {
        size_t ncap = b->cap ? b->cap : 65536;
        while (ncap < b->len + len) ncap *= 2;
        uint8_t *np = (uint8_t *)realloc(b->p, ncap);
        if (np == NULL) return WEM_ERR_INTERNAL;
        b->p = np;
        b->cap = ncap;
    }
    memcpy(b->p + b->len, data, len);
    b->len += len;
    return WEM_OK;
}
*/
import "C"

import (
	"encoding/binary"
	"flag"
	"fmt"
	"os"
	"unsafe"
)

// The one installed exact profile (README: the supported profile table).
const profileName = "wwise2013-6ch-44100"

// wav holds one parsed signed-16 PCM WAV file.
type wav struct {
	sampleRate int
	channels   int
	data       []byte // interleaved little-endian s16 samples
}

// readWav16 parses a minimal RIFF/WAVE PCM-16 file (the kernel's own
// reader semantics: format 1, 16-bit, one fmt chunk, one data chunk).
func readWav16(path string) (*wav, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	if len(raw) < 44 || string(raw[0:4]) != "RIFF" || string(raw[8:12]) != "WAVE" {
		return nil, fmt.Errorf("%s: not a RIFF/WAVE file", path)
	}
	w := &wav{}
	offset := 12
	for offset+8 <= len(raw) {
		id := string(raw[offset : offset+4])
		size := int(binary.LittleEndian.Uint32(raw[offset+4 : offset+8]))
		if offset+8+size > len(raw) {
			return nil, fmt.Errorf("%s: truncated chunk %s", path, id)
		}
		body := raw[offset+8 : offset+8+size]
		switch id {
		case "fmt ":
			if len(body) < 16 {
				return nil, fmt.Errorf("%s: truncated fmt chunk", path)
			}
			if binary.LittleEndian.Uint16(body[0:2]) != 1 {
				return nil, fmt.Errorf("%s: not a PCM (format 1) file", path)
			}
			w.channels = int(binary.LittleEndian.Uint16(body[2:4]))
			w.sampleRate = int(binary.LittleEndian.Uint32(body[4:8]))
			if binary.LittleEndian.Uint16(body[14:16]) != 16 {
				return nil, fmt.Errorf("%s: not a 16-bit file", path)
			}
		case "data":
			w.data = append([]byte(nil), body...)
		}
		offset += 8 + size
	}
	if w.channels == 0 || len(w.data) == 0 {
		return nil, fmt.Errorf("%s: no fmt/data chunks", path)
	}
	return w, nil
}

func main() {
	wavPath := flag.String("wav", "tests/fixtures/input.wav", "input signed-16 PCM WAV file")
	outPath := flag.String("out", "out.wem", "output WEM file")
	flag.Parse()

	w, err := readWav16(*wavPath)
	if err != nil {
		fmt.Fprintln(os.Stderr, "wem-go-cgo:", err)
		os.Exit(1)
	}

	profile := C.CString(profileName)
	defer C.free(unsafe.Pointer(profile))

	var sink C.wem_buf_t
	frames := C.size_t(len(w.data)) / (C.size_t(w.channels) * 2)
	code := C.wem_encode_pcm16_interleaved(
		(*C.char)(profile),
		nil,
		(*C.int16_t)(unsafe.Pointer(&w.data[0])),
		frames,
		C.WemWriteCb(C.wem_buf_write),
		unsafe.Pointer(&sink),
	)
	if code != C.WEM_OK {
		fmt.Fprintf(os.Stderr, "wem-go-cgo: encode failed: WemError %d\n", code)
		os.Exit(1)
	}

	out := C.GoBytes(unsafe.Pointer(sink.p), C.int(sink.len))
	C.free(unsafe.Pointer(sink.p))
	if err := os.WriteFile(*outPath, out, 0o644); err != nil {
		fmt.Fprintln(os.Stderr, "wem-go-cgo:", err)
		os.Exit(1)
	}
	fmt.Printf("wem-go-cgo: %d bytes written to %s (%d frames, %d Hz, %d channels)\n",
		len(out), *outPath, frames, w.sampleRate, w.channels)
}
