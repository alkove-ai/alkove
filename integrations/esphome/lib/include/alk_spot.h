// Spot (alkove-spot-mcu) and the SDK's wake word search, for ESP-IDF; see alkove/integrations/esphome
#pragma once
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct alk_spot alk_spot;

typedef struct {
  uint32_t word;   // index of the word, in the order alk_spot_add_variant first met it
  float score;     // mean log-likelihood ratio per phoneme
  float threshold; // the one it crossed
  float start;     // seconds since the stream started
  float end;
  uint8_t pause;   // 1 after a pause, 0 anywhere
} alk_wake;

// ESP-NN's scratch, in internal RAM, before any alk_spot_new
size_t alk_spot_scratch_size(void);
void alk_spot_set_scratch(void *scratch);

// model: a spot_mcu_export.py file that lives for the whole program, 16-byte aligned; NULL if it does not load
alk_spot *alk_spot_new(const uint8_t *model, size_t len, uint32_t steps_per_call);

// one pronunciation of a word (Lex's phoneme ids); a word is all the variants added under its name
int alk_spot_add_variant(alk_spot *s, const char *word, const uint32_t *ids, size_t n, float p, int french, float anywhere, float pause);

// 0 after a pause, 1 anywhere, 2 both
void alk_spot_set_context(alk_spot *s, int context);

// 16 kHz mono; returns the wakes written to out (at most cap)
size_t alk_spot_push(alk_spot *s, const int16_t *pcm, size_t n, alk_wake *out, size_t cap);

// the best score any word reached since the last call (very low when none was searched)
float alk_spot_take_best(alk_spot *s);

void alk_spot_reset(alk_spot *s);

#ifdef __cplusplus
}
#endif
