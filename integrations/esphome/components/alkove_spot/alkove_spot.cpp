#include "alkove_spot.h"

#include "esphome/core/log.h"

#include <algorithm>

#include <esp_heap_caps.h>
#include <esp_timer.h>

extern "C" {
extern const uint8_t alkove_spot_model[], alkove_spot_model_end[];
extern const uint8_t alkove_spot_test[], alkove_spot_test_end[];
}

namespace esphome {
namespace alkove_spot {

static const char *const TAG = "alkove_spot";

// samples handed to Spot at a time: one 30 ms step is 480
static const size_t BLOCK = 512;
static const size_t RING_BYTES = 16 * 1024;
// ~8 KB measured at most (uxTaskGetStackHighWaterMark), loading included
static const uint32_t STACK = 16 * 1024;

void AlkoveSpot::setup() {
  this->wakes_ = xQueueCreate(4, sizeof(alk_wake));
#ifdef USE_ALKOVE_SPOT_MICROPHONE
  this->ring_buffer_ = ring_buffer::RingBuffer::create(RING_BYTES);
  if (this->ring_buffer_ == nullptr) {
    ESP_LOGE(TAG, "No memory for the audio ring buffer");
    this->mark_failed();
    return;
  }
  std::weak_ptr<ring_buffer::RingBuffer> weak = this->ring_buffer_;
  this->microphone_source_->add_data_callback([weak](const std::vector<uint8_t> &data) {
    if (auto rb = weak.lock())
      rb->write_without_replacement(data.data(), data.size(), 0, false);
  });
#endif
  // the model loads in the task: on ESPHome's loop task stack, loading it overflowed (a crash in the next log line)
  if (xTaskCreatePinnedToCore(AlkoveSpot::task_, "alkove_spot", STACK, this, 3, &this->task_handle_, this->core_) != pdPASS) {
    ESP_LOGE(TAG, "No memory for the task");
    this->mark_failed();
    return;
  }
  this->start();
}

bool AlkoveSpot::load_() {
  void *scratch = heap_caps_aligned_alloc(16, alk_spot_scratch_size() + 16, MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT);
  if (scratch == nullptr) {
    ESP_LOGE(TAG, "No internal RAM for ESP-NN's scratch");
    return false;
  }
  alk_spot_set_scratch(scratch);
  size_t internal = heap_caps_get_free_size(MALLOC_CAP_INTERNAL), psram = heap_caps_get_free_size(MALLOC_CAP_SPIRAM);
  int64_t t = esp_timer_get_time();
  this->spot_ = alk_spot_new(alkove_spot_model, alkove_spot_model_end - alkove_spot_model, this->steps_);
  if (this->spot_ == nullptr) {
    ESP_LOGE(TAG, "The model does not load");
    return false;
  }
  for (auto &v : this->variants_) {
    if (std::find(this->words_.begin(), this->words_.end(), v.word) == this->words_.end())
      this->words_.push_back(v.word);
    alk_spot_add_variant(this->spot_, v.word.c_str(), v.ids.data(), v.ids.size(), v.p, v.french, v.anywhere, v.pause);
  }
  alk_spot_set_context(this->spot_, this->context_);
  ESP_LOGI(TAG, "Spot loaded in %d ms: %u B of PSRAM, %u B of internal RAM", (int) ((esp_timer_get_time() - t) / 1000),
           (unsigned) (psram - heap_caps_get_free_size(MALLOC_CAP_SPIRAM)),
           (unsigned) (internal - heap_caps_get_free_size(MALLOC_CAP_INTERNAL)));
  return true;
}

void AlkoveSpot::start() {
#ifdef USE_ALKOVE_SPOT_MICROPHONE
  if (this->microphone_source_ != nullptr)
    this->microphone_source_->start();
#endif
  this->running_ = true;
}


void AlkoveSpot::stop() {
#ifdef USE_ALKOVE_SPOT_MICROPHONE
  if (this->microphone_source_ != nullptr)
    this->microphone_source_->stop();
#endif
  this->running_ = false;
}

void AlkoveSpot::push_(const int16_t *pcm, size_t n) {
  alk_wake out[4];
  size_t found = alk_spot_push(this->spot_, pcm, n, out, 4);
  for (size_t i = 0; i < found; i++)
    xQueueSend(this->wakes_, &out[i], 0);
  if (this->log_scores_) {
    uint32_t before = this->heard_samples_ / 16000;
    this->heard_samples_ += n;
    if (this->heard_samples_ / 16000 != before) {
      float best = alk_spot_take_best(this->spot_);
      if (best > -3.0f)
        ESP_LOGI(TAG, "Best score %.2f in second %u", best, (unsigned) before);
    }
  }
}

void AlkoveSpot::run_self_test_() {
  const int16_t *pcm = reinterpret_cast<const int16_t *>(alkove_spot_test);
  size_t n = (alkove_spot_test_end - alkove_spot_test) / 2;
  if (n == 0)
    return;
  vTaskDelay(pdMS_TO_TICKS(3000));
  ESP_LOGI(TAG, "Self-test: %.1f s of audio", n / 16000.0f);
  int64_t t = esp_timer_get_time();
  for (size_t at = 0; at < n; at += BLOCK) {
    this->push_(pcm + at, std::min(BLOCK, n - at));
    // as fast as it goes, but the idle task must run: the task watchdog watches it
    vTaskDelay(1);
  }
  float s = (esp_timer_get_time() - t) / 1e6f;
  ESP_LOGI(TAG, "Self-test: %.2f s of compute, %.2fx real time", s, s / (n / 16000.0f));
  alk_spot_reset(this->spot_);
}

void AlkoveSpot::task_(void *arg) {
  auto *self = static_cast<AlkoveSpot *>(arg);
  if (!self->load_()) {
    self->failed_ = true;
    vTaskDelete(nullptr);
  }
  self->loaded_ = true;
  if (self->self_test_)
    self->run_self_test_();
  ESP_LOGI(TAG, "Task stack: %u of %u B never used", (unsigned) uxTaskGetStackHighWaterMark(nullptr), (unsigned) STACK);
#ifdef USE_ALKOVE_SPOT_MICROPHONE
  int16_t block[BLOCK];
  for (;;) {
    size_t got = self->ring_buffer_->read(block, sizeof(block), pdMS_TO_TICKS(100));
    if (got >= 2 && self->running_ && self->loaded_)
      self->push_(block, got / 2);
  }
#else
  for (;;)
    vTaskDelay(portMAX_DELAY);
#endif
}

void AlkoveSpot::loop() {
  if (this->failed_ && !this->is_failed()) {
    this->mark_failed();
    return;
  }
  alk_wake w;
  while (this->wakes_ != nullptr && xQueueReceive(this->wakes_, &w, 0) == pdTRUE) {
    const std::string &word = w.word < this->words_.size() ? this->words_[w.word] : this->words_[0];
    ESP_LOGI(TAG, "Wake \"%s\" at %.2f-%.2f s, score %.3f (threshold %.3f, %s)", word.c_str(), w.start, w.end, w.score,
             w.threshold, w.pause ? "after a pause" : "anywhere");
    this->wake_word_detected_trigger_.trigger(word);
  }
}

void AlkoveSpot::dump_config() {
  ESP_LOGCONFIG(TAG, "Alkove Spot:");
  ESP_LOGCONFIG(TAG, "  Model: %u bytes", (unsigned) (alkove_spot_model_end - alkove_spot_model));
  for (auto &w : this->words_)
    ESP_LOGCONFIG(TAG, "  Word: %s", w.c_str());
  ESP_LOGCONFIG(TAG, "  Steps per call: %u", (unsigned) this->steps_);
}

}  // namespace alkove_spot
}  // namespace esphome
