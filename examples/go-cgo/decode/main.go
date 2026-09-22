// Command wem-go-cgo-decode decodes a WEM container through the same C ABI
// core surface the encode sample uses (include/wem.h section 5): a thin cgo
// shell over wem_decoder_new / _push / _finish / _free. The kernel does all
// the work; this program only moves bytes.
//
// There is no profile selection on this direction: the WEM is self-describing
// and the configuration is resolved from the container's own geometry. The two
// callbacks are required — the one-time header announcement (geometry, declared
// frame count, setup packet) and the PCM delivery — and cgo mirrors their
// signatures 1:1, so the shell adds no container parsing of its own.
//
// The PCM the kernel hands over is interleaved f32 at +-1.0 full scale. This
// command writes those samples raw, as little-endian f32 with no header: the
// file is the decoder's output and not a conversion of it, and a wrong WAV
// writer would look more plausible than it is.
//
// Usage:
//
//	go run ./decode -wem ../../tests/fixtures/reference.wem -out out.f32
//
// Build steps (three lines): examples/go-cgo/README.md
package main

/*
#cgo CFLAGS: -I${SRCDIR}/../../../include
#cgo LDFLAGS: -L${SRCDIR}/../../../crates/target/release -lwem_capi -Wl,-rpath,${SRCDIR}/../../../crates/target/release

#include <wem.h>
#include <stdlib.h>
#include <string.h>

// The decode sink lives in C memory: the kernel passes it back through
// user_data, and a Go pointer holding a Go slice may not cross that boundary.
typedef struct {
    uint8_t *pcm;   // interleaved little-endian f32, the delivered samples
    size_t len;     // bytes written
    size_t cap;
    uint32_t channels;
    uint32_t sample_rate;
    uint64_t total_frames;
    size_t setup_len;
    uint64_t blocks;
    uint64_t frames;
} wem_decode_sink_t;

// Declarations only: the definitions are the //export'ed Go functions below
// (cgo emits their prototypes into _cgo_export.h), so this preamble has to
// agree with the signatures those functions get — non-const pointers — and the
// conversion to WemHeaderCb / WemPcmCb is the usual function-pointer cast.
WemError wemGoHeader(uint32_t channels, uint32_t sample_rate, uint64_t total_frames, uint8_t *setup, size_t setup_len, void *user_data);
WemError wemGoPcm(float *interleaved, size_t frames, void *user_data);
*/
import "C"

import (
	"encoding/binary"
	"flag"
	"fmt"
	"math"
	"os"
	"runtime"
	"unsafe"
)

// pushBytes is the size of one WEM chunk handed to the session. The kernel
// delivers the frames the packets in that push completed, so bounded pushes
// keep both sides bounded; the samples do not depend on the chunking
// (include/wem.h section 5).
const pushBytes = 8192

// wemGoHeader is the one-time header announcement (WemHeaderCb): the geometry,
// the frame count the container declares, and the setup packet this revision
// parsed. It carries what the PCM cannot: the geometry is available nowhere
// else in the decoder's output.
//
//export wemGoHeader
func wemGoHeader(channels C.uint32_t, sampleRate C.uint32_t, totalFrames C.uint64_t, setup *C.uint8_t, setupLen C.size_t, userData unsafe.Pointer) C.WemError {
	sink := (*C.wem_decode_sink_t)(userData)
	if sink == nil || channels == 0 || sampleRate == 0 {
		return C.WEM_ERR_INTERNAL
	}
	sink.channels = channels
	sink.sample_rate = sampleRate
	sink.total_frames = totalFrames
	sink.setup_len = setupLen
	_ = setup // announced; this sample does not print the setup packet
	return C.WEM_OK
}

// wemGoPcm is the PCM delivery (WemPcmCb): interleaved f32 at +-1.0 full
// scale, valid for the duration of the call. The samples are copied out as
// little-endian bytes so the file format does not depend on the host.
//
//export wemGoPcm
func wemGoPcm(interleaved *C.float, frames C.size_t, userData unsafe.Pointer) C.WemError {
	sink := (*C.wem_decode_sink_t)(userData)
	if sink == nil || sink.channels == 0 {
		// PCM before the header announcement cannot be interpreted, and that
		// ordering is the C ABI's promise, so it is a defect rather than a
		// case to guess around.
		return C.WEM_ERR_INTERNAL
	}
	samples := uint64(frames) * uint64(sink.channels)
	need := C.size_t(samples * 4)
	if sink.len+need > sink.cap {
		capacity := sink.cap
		if capacity == 0 {
			capacity = 65536
		}
		for capacity < sink.len+need {
			capacity *= 2
		}
		grown := C.realloc(unsafe.Pointer(sink.pcm), capacity)
		if grown == nil {
			return C.WEM_ERR_INTERNAL
		}
		sink.pcm = (*C.uint8_t)(grown)
		sink.cap = capacity
	}
	// unsafe.Slice over C memory (not Go memory), so no Go pointer crosses the
	// boundary in either direction.
	delivered := unsafe.Slice((*C.float)(unsafe.Pointer(interleaved)), int(samples))
	start := unsafe.Add(unsafe.Pointer(sink.pcm), uintptr(sink.len))
	destination := unsafe.Slice((*byte)(start), int(need))
	for i, sample := range delivered {
		binary.LittleEndian.PutUint32(destination[i*4:], math.Float32bits(float32(sample)))
	}
	sink.len += need
	sink.blocks++
	sink.frames += C.uint64_t(frames)
	return C.WEM_OK
}

func main() {
	wemPath := flag.String("wem", "../../tests/fixtures/reference.wem", "input WEM container")
	outPath := flag.String("out", "out.f32", "output raw interleaved little-endian f32 file")
	flag.Parse()

	wem, err := os.ReadFile(*wemPath)
	if err != nil {
		fmt.Fprintln(os.Stderr, "wem-go-cgo-decode:", err)
		os.Exit(1)
	}

	// The sink is C memory (the kernel holds it across the callbacks) and is
	// released with C.free.
	sink := (*C.wem_decode_sink_t)(C.calloc(1, C.sizeof_wem_decode_sink_t))
	if sink == nil {
		fmt.Fprintln(os.Stderr, "wem-go-cgo-decode: cannot allocate the decode sink")
		os.Exit(1)
	}
	defer func() {
		C.free(unsafe.Pointer(sink.pcm))
		C.free(unsafe.Pointer(sink))
	}()

	// Init -> push* -> Finish -> free, the lifecycle include/wem.h section 5
	// states. Both callbacks are required, and *out_decoder is written on every
	// exit: the handle on WEM_OK, NULL on every failure.
	var decoder *C.WemDecoder
	code := C.wem_decoder_new(
		C.WemHeaderCb(C.wemGoHeader),
		C.WemPcmCb(C.wemGoPcm),
		unsafe.Pointer(sink),
		&decoder,
	)
	if code == C.WEM_OK {
		for offset := 0; offset < len(wem); offset += pushBytes {
			end := offset + pushBytes
			if end > len(wem) {
				end = len(wem)
			}
			// A push that reports any code but WEM_ERR_INTERNAL leaves the
			// handle usable and reports the same rejection again, so this stops
			// at the first refusal instead of looping on it.
			code = C.wem_decoder_push(decoder, (*C.uint8_t)(unsafe.Pointer(&wem[offset])), C.size_t(end-offset))
			if code != C.WEM_OK {
				break
			}
		}
		if code == C.WEM_OK {
			// Terminal whatever it returns; on WEM_OK exactly the declared
			// frame count has been delivered.
			code = C.wem_decoder_finish(decoder)
		}
	}
	C.wem_decoder_free(decoder) // NULL is a no-op
	runtime.KeepAlive(wem)

	if code != C.WEM_OK {
		fmt.Fprintf(os.Stderr, "wem-go-cgo-decode: decode refused: WemError %d\n", code)
		os.Exit(1)
	}
	if uint64(sink.frames) != uint64(sink.total_frames) {
		fmt.Fprintf(os.Stderr, "wem-go-cgo-decode: delivered %d frames, container declares %d\n",
			uint64(sink.frames), uint64(sink.total_frames))
		os.Exit(1)
	}
	// The delivered bytes are already the file's bytes, little-endian f32.
	raw := unsafe.Slice((*byte)(unsafe.Pointer(sink.pcm)), int(sink.len))
	if err := os.WriteFile(*outPath, raw, 0o644); err != nil {
		fmt.Fprintln(os.Stderr, "wem-go-cgo-decode:", err)
		os.Exit(1)
	}
	fmt.Printf("wem-go-cgo-decode: %d frames written to %s (%d Hz, %d channels, container declares %d frames, setup packet %d bytes, %d blocks)\n",
		uint64(sink.frames), *outPath, uint32(sink.sample_rate), uint32(sink.channels),
		uint64(sink.total_frames), uint64(sink.setup_len), uint64(sink.blocks))
}
