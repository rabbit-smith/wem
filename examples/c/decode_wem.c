/* Decode a WEM through the C ABI's decode surface (include/wem.h section 5).
 *
 * Init -> push* -> Finish -> free, with the PCM delivered through the two
 * required callbacks: the one-time header announcement (geometry, declared
 * frame count, setup packet) and the interleaved f32 blocks. The WEM is
 * self-describing, so no profile selection is passed anywhere here.
 *
 * Output: raw interleaved little-endian f32 at +-1.0 full scale, no header —
 * the samples the decoder hands over, written in the one byte order this
 * example states. It is deliberately not a WAV writer: choosing a sample
 * format, a chunk layout and a header would be a surface this example does not
 * own, and one that could be wrong while still looking plausible.
 *
 * Build (from the repository root):
 *   cd crates && cargo build -p wem-capi --release && cd ..
 *   cc -std=c11 -Wall -Wextra -Werror -Iinclude examples/c/decode_wem.c \
 *     -Lcrates/target/release -lwem_capi \
 *     -Wl,-rpath,"$PWD/crates/target/release" -o wem-c-decode
 *   ./wem-c-decode tests/fixtures/reference.wem out.f32
 */

#include <errno.h>
#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <wem.h>

/* The decode surface delivers f32 samples (include/wem.h section 5) and the
 * file written below is four bytes per sample: a host where that is not true
 * must fail to build this example rather than write a file it cannot describe. */
_Static_assert(sizeof(float) == 4, "the decode surface delivers 4-byte f32 samples");

/* WEM bytes handed to the session per push. The kernel delivers the frames the
 * packets in that push completed, so bounded pushes keep both sides bounded;
 * the samples do not depend on the chunking (include/wem.h section 5). */
#define PUSH_BYTES 8192

/* What the two callbacks report back to main. */
typedef struct Sink {
  FILE *output;
  uint32_t channels;      /* 0 until the header announcement arrives */
  uint32_t sample_rate;
  uint64_t total_frames;  /* the container's declared dw_total_pcm_frames */
  size_t setup_len;
  uint64_t blocks;        /* pcm_cb calls that wrote samples */
  uint64_t frames;        /* frames written */
  int announced;          /* header_cb fired (it fires exactly once) */
  int io_failed;          /* a write to OUTPUT failed */
} Sink;

static void usage(const char *program) {
  fprintf(stderr,
          "usage: %s INPUT.wem OUTPUT.f32\n"
          "  INPUT.wem   WEM container to decode\n"
          "  OUTPUT.f32  raw interleaved little-endian f32, +-1.0 full scale,\n"
          "              no header; the geometry and frame counts are printed\n",
          program);
}

static uint8_t *read_file(const char *path, size_t *len) {
  FILE *input = fopen(path, "rb");
  if (input == NULL) return NULL;
  if (fseek(input, 0, SEEK_END) != 0) goto fail;
  long end = ftell(input);
  if (end < 0 || fseek(input, 0, SEEK_SET) != 0) goto fail;
  size_t size = (size_t)end;
  uint8_t *data = malloc(size == 0 ? 1 : size);
  if (data == NULL) goto fail;
  if (fread(data, 1, size, input) != size) {
    free(data);
    data = NULL;
  }
  fclose(input);
  if (data != NULL) *len = size;
  return data;

fail:
  fclose(input);
  return NULL;
}

/* One f32 as four little-endian bytes, whatever the host's own byte order is:
 * the file format is this example's choice, and this is the one place it is
 * made. */
static void pack_f32_le(uint8_t *out, float sample) {
  uint32_t bits = 0;
  memcpy(&bits, &sample, sizeof(bits));
  out[0] = (uint8_t)(bits & 0xFFu);
  out[1] = (uint8_t)((bits >> 8) & 0xFFu);
  out[2] = (uint8_t)((bits >> 16) & 0xFFu);
  out[3] = (uint8_t)((bits >> 24) & 0xFFu);
}

/* The one-time header announcement (wem_header_cb): the geometry, the frame
 * count the container declares, and the setup packet this revision parsed.
 * It carries what the PCM cannot: the geometry is available nowhere else in
 * the decoder's output. */
static WemError announce_header(uint32_t channels, uint32_t sample_rate,
                                uint64_t total_frames, const uint8_t *setup,
                                size_t setup_len, void *user_data) {
  Sink *sink = user_data;
  if (sink->announced) {
    fprintf(stderr, "wem-decode: the header was announced twice\n");
    return WEM_ERR_INTERNAL;
  }
  if (channels == 0 || sample_rate == 0) {
    fprintf(stderr, "wem-decode: the header announced a zero geometry\n");
    return WEM_ERR_INTERNAL;
  }
  sink->announced = 1;
  sink->channels = channels;
  sink->sample_rate = sample_rate;
  sink->total_frames = total_frames;
  sink->setup_len = setup_len;
  (void)setup; /* the setup packet is announced; this example does not print it */
  return WEM_OK;
}

/* PCM delivery (wem_pcm_cb): interleaved f32 at +-1.0 full scale, in bounded
 * blocks, valid only for the duration of the call. */
static WemError write_pcm(const float *interleaved, size_t frames,
                          void *user_data) {
  Sink *sink = user_data;
  if (sink->channels == 0) {
    /* The header announcement fires before any PCM; PCM without a geometry
     * cannot be interpreted, so this is a defect and not something to guess
     * around. */
    fprintf(stderr, "wem-decode: PCM arrived before the header announcement\n");
    return WEM_ERR_INTERNAL;
  }
  size_t samples = frames * (size_t)sink->channels;
  uint8_t buffer[4096];
  size_t done = 0;
  while (done < samples) {
    size_t capacity = sizeof(buffer) / 4;
    size_t take = samples - done < capacity ? samples - done : capacity;
    for (size_t i = 0; i < take; i++) {
      pack_f32_le(buffer + i * 4, interleaved[done + i]);
    }
    if (fwrite(buffer, 4, take, sink->output) != take) {
      /* A callback can only abort with a WemError code; the failure is the
       * output file's, and main reports it as such. */
      sink->io_failed = 1;
      return WEM_ERR_INTERNAL;
    }
    done += take;
  }
  sink->blocks += 1;
  sink->frames += frames;
  return WEM_OK;
}

int main(int argc, char **argv) {
  const char *input_path = NULL;
  const char *output_path = NULL;

  for (int i = 1; i < argc; i++) {
    const char *arg = argv[i];
    if (strcmp(arg, "--help") == 0 || strcmp(arg, "-h") == 0) {
      usage(argv[0]);
      return 0;
    }
    if (arg[0] == '-' && arg[1] != '\0') {
      fprintf(stderr, "unknown option: %s\n", arg);
      usage(argv[0]);
      return 2;
    }
    if (input_path == NULL) {
      input_path = arg;
    } else if (output_path == NULL) {
      output_path = arg;
    } else {
      fprintf(stderr, "unexpected argument: %s\n", arg);
      usage(argv[0]);
      return 2;
    }
  }
  if (input_path == NULL || output_path == NULL) {
    usage(argv[0]);
    return 2;
  }

  size_t input_len = 0;
  uint8_t *input = read_file(input_path, &input_len);
  if (input == NULL) {
    fprintf(stderr, "read %s: %s\n", input_path, strerror(errno));
    return 1;
  }

  FILE *output = fopen(output_path, "wb");
  if (output == NULL) {
    fprintf(stderr, "open %s: %s\n", output_path, strerror(errno));
    free(input);
    return 1;
  }

  Sink sink;
  memset(&sink, 0, sizeof(sink)); /* channels == 0: nothing announced yet */
  sink.output = output;

  /* Init: both callbacks are required (the geometry is available nowhere else
   * in the decoder's output). `*out_decoder` is written on every exit: the
   * handle on WEM_OK, NULL on every failure. */
  WemDecoder *decoder = NULL;
  WemError error = wem_decoder_new(announce_header, write_pcm, &sink, &decoder);
  if (error == WEM_OK) {
    /* chunk*: the bytes of the WEM, in bounded pushes. A push that reports any
     * code but WEM_ERR_INTERNAL leaves the handle usable and reports the same
     * rejection again, so this stops at the first refusal instead of looping
     * on it. */
    for (size_t offset = 0; offset < input_len; offset += PUSH_BYTES) {
      size_t take = input_len - offset < PUSH_BYTES ? input_len - offset : PUSH_BYTES;
      error = wem_decoder_push(decoder, input + offset, take);
      if (error != WEM_OK) break;
    }
    if (error == WEM_OK) {
      /* Finish: terminal whatever it returns. On WEM_OK the session has
       * delivered exactly the declared frame count. */
      error = wem_decoder_finish(decoder);
    }
  }
  wem_decoder_free(decoder); /* NULL is a no-op */
  int close_error = fclose(output);
  free(input);

  if (sink.io_failed) {
    fprintf(stderr, "write %s: %s\n", output_path, strerror(errno));
    return 1;
  }
  if (error != WEM_OK) {
    fprintf(stderr, "decode %s refused: WemError %d\n", input_path, error);
    return 1;
  }
  if (close_error != 0) {
    fprintf(stderr, "close %s: %s\n", output_path, strerror(errno));
    return 1;
  }
  if (sink.frames != sink.total_frames) {
    fprintf(stderr,
            "delivered %" PRIu64 " frames, container declares %" PRIu64 "\n",
            sink.frames, sink.total_frames);
    return 1;
  }
  printf("wrote %s (%" PRIu64 " frames, %u channels, %u Hz, container declares "
         "%" PRIu64 " frames, setup packet %zu bytes, %" PRIu64 " blocks)\n",
         output_path, sink.frames, sink.channels, sink.sample_rate,
         sink.total_frames, sink.setup_len, sink.blocks);
  return 0;
}