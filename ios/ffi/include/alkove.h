// kept by hand: the header test in ../src/lib.rs compares each declaration with its export
#ifndef ALKOVE_H
#define ALKOVE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

typedef struct AlkModel AlkModel;
typedef struct AlkChain AlkChain;
typedef struct AlkLive AlkLive;
typedef struct AlkTracker AlkTracker;
typedef struct AlkProcessor AlkProcessor;
typedef struct AlkResampler AlkResampler;
typedef struct AlkLex AlkLex;
typedef struct AlkListener AlkListener;
typedef struct AlkEnroller AlkEnroller;
typedef struct AlkTimbre AlkTimbre;
typedef struct AlkLearner AlkLearner;
typedef struct AlkPipeline AlkPipeline;
typedef struct AlkCoach AlkCoach;

typedef struct AlkBytes {
    uint8_t *ptr;
    size_t len;
} AlkBytes;

typedef struct AlkFloats {
    float *ptr;
    size_t len;
} AlkFloats;

typedef struct AlkError {
    int32_t kind;
    AlkBytes message;
} AlkError;

void alk_bytes_free(AlkBytes b);
void alk_floats_free(AlkFloats x);

AlkBytes alk_info(AlkError *err);

const AlkModel *alk_model_load(const uint8_t *p, size_t n, AlkError *err);
void alk_model_free(const AlkModel *m);

AlkChain *alk_chain_new(const AlkModel *focus, const AlkModel *cue, const uint8_t *options, size_t options_len, AlkError *err);
void alk_chain_free(AlkChain *h);
int32_t alk_chain_step(AlkChain *h, const float *pcm, size_t n, float *output, size_t output_cap, double *events, size_t events_cap, float *focus, size_t focus_cap, AlkError *err);
void alk_chain_set_print(AlkChain *h, const float *print, size_t n, AlkError *err);
void alk_chain_set_strength(AlkChain *h, double s, AlkError *err);
void alk_chain_set_enabled(AlkChain *h, bool on, AlkError *err);
void alk_chain_listen(AlkChain *h, bool on, AlkError *err);
void alk_chain_hold(AlkChain *h, bool on, AlkError *err);
double alk_chain_get(AlkChain *h, int32_t what, AlkError *err);

AlkLive *alk_live_new(const AlkModel *focus, const AlkModel *cue, double rate, const uint8_t *options, size_t options_len, AlkError *err);
void alk_live_free(AlkLive *h);
size_t alk_live_push(AlkLive *h, const float *pcm, size_t n, float *output, size_t output_cap, size_t *events, AlkError *err);
const double *alk_live_events(AlkLive *h, size_t *len, AlkError *err);
void alk_live_set_print(AlkLive *h, const float *print, size_t n, AlkError *err);
void alk_live_set_strength(AlkLive *h, double s, AlkError *err);
void alk_live_set_enabled(AlkLive *h, bool on, AlkError *err);
void alk_live_listen(AlkLive *h, bool on, AlkError *err);
void alk_live_hold(AlkLive *h, bool on, AlkError *err);
double alk_live_get(AlkLive *h, int32_t what, AlkError *err);

AlkTracker *alk_tracker_new(double confidence, double exit_confidence, double start_secs, double stop_secs, double frame_secs, AlkError *err);
void alk_tracker_free(AlkTracker *h);
int32_t alk_tracker_push(AlkTracker *h, double p, int64_t k, double *output, AlkError *err);
void alk_tracker_reset(AlkTracker *h, AlkError *err);
bool alk_tracker_get(AlkTracker *h, int32_t what, AlkError *err);

AlkProcessor *alk_processor_new(const AlkModel *focus, const AlkModel *cue, const AlkModel *spot, const uint8_t *options, size_t options_len, AlkError *err);
void alk_processor_free(AlkProcessor *h);
AlkBytes alk_processor_push(AlkProcessor *h, const float *pcm, size_t n, AlkError *err);
AlkBytes alk_processor_end(AlkProcessor *h, AlkError *err);
AlkBytes alk_process(const AlkModel *focus, const AlkModel *cue, const AlkModel *spot, const float *pcm, size_t n, const uint8_t *options, size_t options_len, AlkError *err);

AlkFloats alk_resample(const float *pcm, size_t n, double in_rate, double out_rate, AlkError *err);
AlkResampler *alk_resampler_new(double in_rate, double out_rate, bool aligned, AlkError *err);
void alk_resampler_free(AlkResampler *h);
const float *alk_resampler_push(AlkResampler *h, const float *pcm, size_t n, size_t *len, AlkError *err);

AlkLex *alk_lex_new(const AlkModel *m, AlkError *err);
void alk_lex_free(AlkLex *h);
AlkBytes alk_lex_phonemize(AlkLex *h, const uint8_t *args, size_t args_len, AlkError *err);
AlkBytes alk_compile(AlkLex *lex, const uint8_t *args, size_t args_len, AlkError *err);
AlkBytes alk_word(const uint8_t *word, size_t word_len, AlkError *err);
double alk_distance(const uint8_t *args, size_t args_len, AlkError *err);

AlkListener *alk_listener_new(const AlkModel *spot, const uint8_t *args, size_t args_len, AlkError *err);
void alk_listener_free(AlkListener *h);
void alk_listener_words(AlkListener *h, const uint8_t *args, size_t args_len, AlkError *err);
AlkBytes alk_listener_push(AlkListener *h, const float *pcm, size_t n, AlkError *err);

AlkEnroller *alk_enroller_new(const AlkModel *spot, const uint8_t *args, size_t args_len, AlkError *err);
void alk_enroller_free(AlkEnroller *h);
AlkBytes alk_enroller_push(AlkEnroller *h, const float *pcm, size_t n, AlkError *err);
AlkBytes alk_enroller_result(AlkEnroller *h, AlkError *err);

AlkTimbre *alk_timbre_new(const AlkModel *m, AlkError *err);
void alk_timbre_free(AlkTimbre *h);
AlkBytes alk_timbre_windows(AlkTimbre *h, const float *pcm, size_t n, AlkError *err);
AlkBytes alk_timbre_embed(AlkTimbre *h, const float *pcm, size_t n, const uint8_t *args, size_t args_len, AlkError *err);
AlkBytes alk_voiceprint(const uint8_t *op, size_t op_len, AlkError *err);

AlkLearner *alk_learner_new(const uint8_t *args, size_t args_len, AlkError *err);
void alk_learner_free(AlkLearner *h);
AlkBytes alk_learner_learn(AlkLearner *h, AlkTimbre *timbre, const float *pcm, size_t n, AlkError *err);
AlkBytes alk_learner_offer(AlkLearner *h, AlkTimbre *timbre, const float *pcm, size_t n, AlkError *err);
AlkBytes alk_learner_voiceprint(AlkLearner *h, bool raw, AlkError *err);
AlkBytes alk_learner_state(AlkLearner *h, AlkError *err);
void alk_learner_forget(AlkLearner *h, AlkError *err);

AlkPipeline *alk_pipeline_new(const AlkModel *focus, const AlkModel *cue, const AlkModel *spot, const uint8_t *options, size_t options_len, AlkError *err);
void alk_pipeline_free(AlkPipeline *h);
int32_t alk_pipeline_step(AlkPipeline *h, const float *pcm, size_t n, double far_quiet_ms, float *output, size_t output_cap, AlkError *err);
int32_t alk_pipeline_raw(AlkPipeline *h, const float *pcm, size_t n, AlkError *err);
AlkBytes alk_pipeline_outs(AlkPipeline *h, AlkError *err);
void alk_pipeline_spot(AlkPipeline *h, const AlkModel *m, AlkError *err);
void alk_pipeline_set(AlkPipeline *h, const uint8_t *change, size_t change_len, AlkError *err);
void alk_pipeline_verified(AlkPipeline *h, uint32_t key, bool same, double score, AlkError *err);
void alk_pipeline_record(AlkPipeline *h, double seconds, AlkError *err);
AlkBytes alk_pipeline_recent(AlkPipeline *h, AlkError *err);
AlkBytes alk_pipeline_stats(AlkPipeline *h, AlkError *err);
double alk_pipeline_get(AlkPipeline *h, int32_t what, AlkError *err);

AlkCoach *alk_coach_new(const uint8_t *args, size_t args_len, AlkError *err);
void alk_coach_free(AlkCoach *h);
double alk_coach_learner(AlkCoach *h, const uint8_t *args, size_t args_len, AlkError *err);
void alk_coach_forget(AlkCoach *h, AlkError *err);
AlkBytes alk_coach_window(AlkCoach *h, AlkTimbre *timbre, const float *pcm, size_t n, double far_quiet_ms, double t, AlkError *err);
AlkBytes alk_coach_verify(AlkCoach *h, AlkTimbre *timbre, const float *pcm, size_t n, AlkError *err);

AlkBytes alk_json_stringify(const uint8_t *v, size_t v_len, AlkError *err);
AlkBytes alk_json_parse(const uint8_t *text, size_t text_len, AlkError *err);

#endif
