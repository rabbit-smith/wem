/* Encode signed-16 PCM through the WEM C ABI core surface.
 *
 * ABI revision 2 selects the encoder configuration with one structured
 * `WemProfile` value (a Wwise generation plus the PCM geometry), passed by
 * pointer. A RIFF/WAVE input describes its own geometry, so this example
 * reads channels and sample rate out of the header; header-less PCM falls
 * back to an explicit `--channels` / `--sample-rate` selection.
 */

#include <errno.h>
#include <limits.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <wem.h>

/* The Wwise generation this example selects (include/wem.h `WemVersion`;
 * the revision 2 table holds exactly this one code). */
static const WemVersion kWwiseVersion = WEM_WWISE_2013;

static void usage(const char *program) {
  fprintf(stderr,
          "usage: %s INPUT OUTPUT [--channels N] [--sample-rate HZ]\n"
          "  INPUT   signed-16 PCM RIFF/WAVE file, or header-less\n"
          "          interleaved little-endian signed-16 PCM\n"
          "  OUTPUT  WEM container to write\n"
          "  A RIFF/WAVE input selects its own geometry; --channels and\n"
          "  --sample-rate are the explicit fallback for header-less PCM.\n",
          program);
}

static WemError write_file(const uint8_t *data, size_t len, void *user_data) {
  FILE *output = user_data;
  return fwrite(data, 1, len, output) == len ? WEM_OK : WEM_ERR_INTERNAL;
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

static uint16_t read_u16le(const uint8_t *p) {
  return (uint16_t)((uint16_t)p[0] | ((uint16_t)p[1] << 8));
}

static uint32_t read_u32le(const uint8_t *p) {
  return (uint32_t)p[0] | ((uint32_t)p[1] << 8) | ((uint32_t)p[2] << 16) |
         ((uint32_t)p[3] << 24);
}

/* What sniffing the input for a RIFF/WAVE container found. */
typedef enum WavParse {
  WAV_PARSE_OK = 0,       /* signed-16 PCM payload + geometry found */
  WAV_PARSE_NOT_WAVE = 1, /* no RIFF/WAVE magic: header-less PCM input */
  WAV_PARSE_INVALID = 2   /* RIFF/WAVE-shaped, but not signed-16 PCM */
} WavParse;

/* Parse a minimal RIFF/WAVE signed-16 PCM file: format 1, one `fmt ` chunk
 * and one `data` chunk (the kernel's own reader semantics). On
 * WAV_PARSE_OK, `*pcm` points into `raw` and `*profile` carries the PCM
 * geometry the header describes. */
static WavParse parse_wav(const uint8_t *raw, size_t len, const uint8_t **pcm,
                          size_t *pcm_len, WemProfile *profile) {
  if (len < 12 || memcmp(raw, "RIFF", 4) != 0 ||
      memcmp(raw + 8, "WAVE", 4) != 0) {
    return WAV_PARSE_NOT_WAVE;
  }
  const uint8_t *data = NULL;
  size_t data_len = 0;
  int32_t channels = 0;
  int32_t sample_rate = 0;
  size_t offset = 12;
  while (offset + 8 <= len) {
    uint32_t size = read_u32le(raw + offset + 4);
    if ((size_t)size > len - offset - 8) return WAV_PARSE_INVALID;
    const uint8_t *body = raw + offset + 8;
    if (memcmp(raw + offset, "fmt ", 4) == 0) {
      if (size < 16 || read_u16le(body) != 1 || read_u16le(body + 14) != 16) {
        return WAV_PARSE_INVALID;
      }
      uint32_t rate = read_u32le(body + 4);
      if (rate > (uint32_t)INT32_MAX) return WAV_PARSE_INVALID;
      channels = (int32_t)read_u16le(body + 2);
      sample_rate = (int32_t)rate;
    } else if (memcmp(raw + offset, "data", 4) == 0) {
      data = body;
      data_len = (size_t)size;
    }
    /* RIFF chunk bodies are padded to an even length. */
    offset += 8 + (size_t)size + ((size_t)size & 1u);
  }
  if (channels <= 0 || sample_rate <= 0 || data == NULL) {
    return WAV_PARSE_INVALID;
  }
  profile->version = kWwiseVersion;
  profile->channels = channels;
  profile->sample_rate = sample_rate;
  *pcm = data;
  *pcm_len = data_len;
  return WAV_PARSE_OK;
}

/* Parse a strictly positive decimal integer that fits an int32_t. */
static int parse_positive(const char *text, long *out) {
  if (*text == '\0') return -1;
  char *end = NULL;
  errno = 0;
  long value = strtol(text, &end, 10);
  if (errno == ERANGE || *end != '\0' || value <= 0 || value > INT32_MAX) {
    return -1;
  }
  *out = value;
  return 0;
}

int main(int argc, char **argv) {
  const char *input_path = NULL;
  const char *output_path = NULL;
  long channels = 0;    /* 0 = not given on the command line */
  long sample_rate = 0; /* 0 = not given on the command line */

  for (int i = 1; i < argc; i++) {
    const char *arg = argv[i];
    if (strcmp(arg, "--help") == 0 || strcmp(arg, "-h") == 0) {
      usage(argv[0]);
      return 0;
    }
    if (strcmp(arg, "--channels") == 0 || strcmp(arg, "--sample-rate") == 0) {
      if (i + 1 >= argc) {
        fprintf(stderr, "%s needs a value\n", arg);
        return 2;
      }
      long *target =
          strcmp(arg, "--channels") == 0 ? &channels : &sample_rate;
      if (parse_positive(argv[++i], target) != 0) {
        fprintf(stderr, "%s: %s must be a positive integer\n", arg, argv[i]);
        return 2;
      }
    } else if (arg[0] == '-' && arg[1] != '\0') {
      fprintf(stderr, "unknown option: %s\n", arg);
      usage(argv[0]);
      return 2;
    } else if (input_path == NULL) {
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
  if (input_len == 0) {
    fprintf(stderr, "read %s: empty input\n", input_path);
    free(input);
    return 1;
  }

  /* One structured selection: the header's geometry, or the explicit
   * fallback for header-less PCM. */
  WemProfile profile;
  const uint8_t *pcm = input;
  size_t pcm_len = input_len;
  WavParse parsed = parse_wav(input, input_len, &pcm, &pcm_len, &profile);
  if (parsed == WAV_PARSE_INVALID) {
    fprintf(stderr, "%s: not a signed-16 PCM RIFF/WAVE file\n", input_path);
    free(input);
    return 1;
  }
  if (parsed == WAV_PARSE_NOT_WAVE) {
    if (channels == 0 || sample_rate == 0) {
      fprintf(stderr,
              "%s: no RIFF/WAVE header: pass --channels and --sample-rate\n",
              input_path);
      free(input);
      return 2;
    }
    profile.version = kWwiseVersion;
    profile.channels = (int32_t)channels;
    profile.sample_rate = (int32_t)sample_rate;
  } else if ((channels != 0 && channels != profile.channels) ||
             (sample_rate != 0 && sample_rate != profile.sample_rate)) {
    fprintf(stderr, "%s: explicit geometry disagrees with the WAV header\n",
            input_path);
    free(input);
    return 2;
  }

  if (pcm_len == 0) {
    fprintf(stderr, "%s: no PCM payload\n", input_path);
    free(input);
    return 1;
  }

  /* RIFF pads chunk bodies to even lengths, so the s16 view is aligned. */
  size_t bytes_per_frame = (size_t)profile.channels * sizeof(int16_t);
  if (pcm_len % bytes_per_frame != 0) {
    fprintf(stderr, "input length is not a whole number of PCM frames\n");
    free(input);
    return 1;
  }
  size_t frames = pcm_len / bytes_per_frame;

  FILE *output = fopen(output_path, "wb");
  if (output == NULL) {
    fprintf(stderr, "open %s: %s\n", output_path, strerror(errno));
    free(input);
    return 1;
  }

  WemError error = wem_encode_pcm16_interleaved(
      &profile, (const int16_t *)pcm, frames, write_file, output);
  int close_error = fclose(output);
  free(input);
  if (error != WEM_OK || close_error != 0) {
    fprintf(stderr, "encode failed: WemError %d\n", error);
    return 1;
  }
  printf("wrote %s (%zu frames, %d channels, %d Hz)\n", output_path, frames,
         profile.channels, profile.sample_rate);
  return 0;
}