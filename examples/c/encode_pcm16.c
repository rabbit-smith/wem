/* Encode interleaved little-endian signed-16 PCM through the WEM C ABI. */

#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <wem.h>

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
  uint8_t *data = malloc((size_t)end);
  if (data == NULL && end != 0) goto fail;
  if (fread(data, 1, (size_t)end, input) != (size_t)end) {
    free(data);
    data = NULL;
  }
  fclose(input);
  if (data != NULL) *len = (size_t)end;
  return data;

fail:
  fclose(input);
  return NULL;
}

int main(int argc, char **argv) {
  if (argc != 5) {
    fprintf(stderr,
            "usage: %s INPUT.pcm OUTPUT.wem PROFILE CHANNELS\n",
            argv[0]);
    return 2;
  }

  char *end = NULL;
  errno = 0;
  unsigned long channels = strtoul(argv[4], &end, 10);
  if (errno == ERANGE || *argv[4] == '\0' || *end != '\0' || channels == 0 ||
      channels > SIZE_MAX / sizeof(int16_t)) {
    fprintf(stderr, "channels must be a positive integer\n");
    return 2;
  }
  size_t bytes_per_frame = (size_t)channels * sizeof(int16_t);

  size_t input_len = 0;
  uint8_t *input = read_file(argv[1], &input_len);
  if (input == NULL) {
    fprintf(stderr, "read %s: %s\n", argv[1], strerror(errno));
    return 1;
  }
  if (input_len % bytes_per_frame != 0) {
    fprintf(stderr, "input length is not a whole number of PCM frames\n");
    free(input);
    return 1;
  }

  FILE *output = fopen(argv[2], "wb");
  if (output == NULL) {
    fprintf(stderr, "open %s: %s\n", argv[2], strerror(errno));
    free(input);
    return 1;
  }

  size_t frames = input_len / bytes_per_frame;
  WemError error = wem_encode_pcm16_interleaved(
      argv[3], NULL, (const int16_t *)input, frames, write_file, output);
  int close_error = fclose(output);
  free(input);
  if (error != WEM_OK || close_error != 0) {
    fprintf(stderr, "encode failed: WemError %d\n", error);
    return 1;
  }
  printf("wrote %s (%zu frames)\n", argv[2], frames);
  return 0;
}
