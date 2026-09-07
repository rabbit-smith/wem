// wwise.v1 reference gRPC client (Go).
//
// Streams a local WAV through WemEncoder.Encode on the tonic server
// (crates/wem-server) and writes the returned WEM container:
//
//   go run . -addr 127.0.0.1:50051 -wav tests/fixtures/input.wav -out out.wem
//
// The expected sha256 for tests/fixtures/input.wav (6ch/44100, signed-16
// interleaved) is the golden WEM hash pinned by the Rust kernel tests:
//
//   17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247

package main

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"flag"
	"fmt"
	"io"
	"os"
	"time"

	v1 "grpc-go/wwise/v1"

	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
)

const (
	defaultAddr  = "127.0.0.1:50051"
	defaultChunk = 4096 * 6 * 2 // bytes; frame alignment is re-checked below
)

func envOr(key, fallback string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return fallback
}

func main() {
	addr := flag.String("addr", envOr("WEM_GRPC_ADDR", defaultAddr), "gRPC address (host:port)")
	wavPath := flag.String("wav", "", "input WAV path (signed-16 interleaved PCM)")
	outPath := flag.String("out", "out.wem", "output WEM path")
	chunkBytes := flag.Int("chunk-bytes", defaultChunk, "bytes per streamed chunk")
	flag.Parse()

	if *wavPath == "" {
		fatal("-wav is required")
	}

	// 1) Read the PCM payload (and geometry) from the WAV file.
	pcm, channels, sampleRate := readWav16(*wavPath)
	frameBytes := int(channels) * 2
	if *chunkBytes%frameBytes != 0 {
		*chunkBytes -= *chunkBytes % frameBytes // force frame alignment
	}
	fmt.Printf("input: %s (%d bytes PCM, %d ch, %d Hz)\n", *wavPath, len(pcm), channels, sampleRate)

	// 2) Connect and resolve the installed profile for this geometry.
	conn, err := grpc.NewClient(*addr, grpc.WithTransportCredentials(insecure.NewCredentials()))
	if err != nil {
		fatal(err.Error())
	}
	defer conn.Close()
	client := v1.NewWemEncoderClient(conn)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()

	profilesResp, err := client.ListProfiles(ctx, &v1.ListProfilesRequest{
		Key: &v1.ProfileKey{Channels: channels, SampleRate: sampleRate},
	})
	if err != nil {
		fatal("ListProfiles: " + err.Error())
	}
	if len(profilesResp.Profiles) == 0 {
		fatal("no installed profile for the requested geometry")
	}
	profile := profilesResp.Profiles[0]
	fmt.Printf("profile: %s (setup_sha256=%s manifest_sha256=%s)\n",
		profile.Name, profile.SetupSha256, profile.ManifestSha256)

	// 3) Stream Init -> chunks -> Finish and consume the reply stream.
	stream, err := client.Encode(ctx)
	if err != nil {
		fatal("Encode open: " + err.Error())
	}
	initReq := &v1.EncodeRequest{
		Payload: &v1.EncodeRequest_Init{
			Init: &v1.Init{
				Ref: &v1.Init_Profile{
					Profile: &v1.ProfileRef{
						SetupSha256: profile.SetupSha256,
						Name:        profile.Name, // soft cross-check
					},
				},
			},
		},
	}
	if err := stream.Send(initReq); err != nil {
		fatal("send Init: " + err.Error())
	}
	var packets [][]byte
	complete := &v1.WemComplete{}
	haveComplete := false
	format := &v1.PcmFormat{
		Channels:   channels,
		SampleRate: sampleRate,
		Layout:     v1.PcmSampleLayout_PCM_SAMPLE_LAYOUT_SIGNED_16_INTERLEAVED,
	}
	for offset := 0; offset < len(pcm); {
		end := offset + *chunkBytes
		if end > len(pcm) {
			end = len(pcm)
		}
		req := &v1.EncodeRequest{
			Payload: &v1.EncodeRequest_Chunk{
				Chunk: &v1.PcmChunk{
					Format: format,
					Frames: &v1.PcmFrames{Data: pcm[offset:end]},
				},
			},
		}
		if err := stream.Send(req); err != nil {
			fatal("send chunk: " + err.Error())
		}
		offset = end
	}
	if err := stream.Send(&v1.EncodeRequest{
		Payload: &v1.EncodeRequest_Finish{Finish: &v1.Finish{}},
	}); err != nil {
		fatal("send Finish: " + err.Error())
	}

	for {
		resp, err := stream.Recv()
		if err != nil {
			if err == io.EOF {
				break
			}
			fatal("recv: " + err.Error())
		}
		switch p := resp.Payload.(type) {
		case *v1.EncodeResponse_Packet:
			if int(p.Packet.Seq) != len(packets) {
				fatal(fmt.Sprintf("packet seq out of order: got %d, want %d", p.Packet.Seq, len(packets)))
			}
			packets = append(packets, p.Packet.Data)
		case *v1.EncodeResponse_Complete:
			complete = p.Complete
			haveComplete = true
		case *v1.EncodeResponse_Error:
			fatal(fmt.Sprintf("server error: %v (%s)", p.Error.Code, p.Error.Message))
		default:
			fatal("empty EncodeResponse")
		}
	}
	if !haveComplete {
		fatal("stream closed without WemComplete")
	}
	fmt.Printf("packets: %d (setup + audio), total_len=%d, sha256=%s, inline=%d bytes\n",
		len(packets), complete.TotalLen, complete.Sha256, len(complete.InlineBytes))

	// 4) Self-check and write the container.
	if len(complete.InlineBytes) == 0 {
		fatal("server left inline_bytes empty (output > 2 MiB); the reference " +
			"client reconstructs containers only through inline_bytes (TODO: " +
			"implement packet-stream reconstruction in Go for large outputs)")
	}
	got := sha256.Sum256(complete.InlineBytes)
	if hex.EncodeToString(got[:]) != complete.Sha256 {
		fatal("inline bytes sha256 mismatch")
	}
	if err := os.WriteFile(*outPath, complete.InlineBytes, 0o644); err != nil {
		fatal("write output: " + err.Error())
	}
	fmt.Printf("wrote: %s\n", *outPath)
	fmt.Printf("sha256: %s\n", hex.EncodeToString(got[:]))
}

func fatal(message string) {
	fmt.Fprintln(os.Stderr, "error:", message)
	os.Exit(1)
}

// readWav16 parses a minimal RIFF/WAVE file with one fmt chunk and one
// data chunk (the repository fixture layout) and returns the interleaved
// signed-16 PCM plus the geometry.
func readWav16(path string) ([]byte, uint32, uint32) {
	f, err := os.Open(path)
	if err != nil {
		fatal("open wav: " + err.Error())
	}
	defer f.Close()
	return parseWav16(f)
}

func parseWav16(r io.Reader) ([]byte, uint32, uint32) {
	// RIFF....WAVE
	head := make([]byte, 12)
	if _, err := io.ReadFull(r, head); err != nil {
		fatal("read wav header: " + err.Error())
	}
	if string(head[0:4]) != "RIFF" || string(head[8:12]) != "WAVE" {
		fatal("not a RIFF/WAVE file")
	}
	var channels, sampleRate uint32
	var pcm []byte
	for {
		chunkHead := make([]byte, 8)
		n, err := io.ReadFull(r, chunkHead)
		if err != nil || n == 0 {
			break
		}
		id := string(chunkHead[0:4])
		size := int32(chunkHead[4]) | int32(chunkHead[5])<<8 | int32(chunkHead[6])<<16 | int32(chunkHead[7])<<24
		payload := make([]byte, size)
		if _, err := io.ReadFull(r, payload); err != nil {
			fatal("read wav chunk " + id + ": " + err.Error())
		}
		switch id {
		case "fmt ":
			if size < 16 {
				fatal("fmt chunk too short")
			}
			channels = uint32(payload[2]) | uint32(payload[3])<<8
			sampleRate = uint32(payload[4]) | uint32(payload[5])<<8 | uint32(payload[6])<<16 | uint32(payload[7])<<24
		case "data":
			pcm = append(pcm, payload...)
		}
		// RIFF chunks are word-aligned.
		if size%2 == 1 {
			if _, err := r.Read(make([]byte, 1)); err != nil {
				break
			}
		}
	}
	if channels == 0 || sampleRate == 0 || len(pcm) == 0 {
		fatal("no fmt/data chunks found")
	}
	return pcm, channels, sampleRate
}
