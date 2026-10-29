#pragma once

#include "esphome/core/automation.h"
#include "esphome/core/component.h"
#include "esphome/core/defines.h"

#ifdef USE_ALKOVE_SPOT_MICROPHONE
#include "esphome/components/microphone/microphone_source.h"
#include "esphome/components/ring_buffer/ring_buffer.h"
#endif

#include <freertos/FreeRTOS.h>
#include <freertos/queue.h>
#include <freertos/task.h>

#include <memory>
#include <string>
#include <vector>

#include "alk_spot.h"

namespace esphome {
namespace alkove_spot {

// Spot listening for typed wake words (alkove-spot-mcu through alkove_spot.h), in its own task
class AlkoveSpot : public Component {
 public:
  void setup() override;
  void loop() override;
  float get_setup_priority() const override { return setup_priority::AFTER_CONNECTION; }
  void dump_config() override;

  void add_variant(const std::string &word, std::vector<uint32_t> ids, float p, bool french, float anywhere, float pause) {
    this->variants_.push_back({word, std::move(ids), p, french, anywhere, pause});
  }
  void set_context(int context) { this->context_ = context; }
  void set_steps_per_call(uint32_t steps) { this->steps_ = steps; }
  void set_task_core(int core) { this->core_ = core; }
  void set_self_test(bool on) { this->self_test_ = on; }
#ifdef USE_ALKOVE_SPOT_MICROPHONE
  void set_microphone_source(microphone::MicrophoneSource *source) { this->microphone_source_ = source; }
#endif

  void start();
  void stop();
  bool is_running() const { return this->running_; }

  Trigger<std::string> *get_wake_word_detected_trigger() { return &this->wake_word_detected_trigger_; }

 protected:
  struct VariantConf {
    std::string word;
    std::vector<uint32_t> ids;
    float p;
    bool french;
    float anywhere;
    float pause;
  };

  static void task_(void *arg);
  bool load_();
  void run_self_test_();
  void push_(const int16_t *pcm, size_t n);

  std::vector<VariantConf> variants_;
  std::vector<std::string> words_;
  int context_{0};
  uint32_t steps_{8};
  int core_{0};
  bool self_test_{false};
  bool running_{false};
  volatile bool loaded_{false};
  volatile bool failed_{false};

  alk_spot *spot_{nullptr};
  QueueHandle_t wakes_{nullptr};
  TaskHandle_t task_handle_{nullptr};
  Trigger<std::string> wake_word_detected_trigger_;

#ifdef USE_ALKOVE_SPOT_MICROPHONE
  microphone::MicrophoneSource *microphone_source_{nullptr};
  std::shared_ptr<ring_buffer::RingBuffer> ring_buffer_;
#endif
};

template<typename... Ts> class StartAction : public Action<Ts...>, public Parented<AlkoveSpot> {
 public:
  void play(const Ts &...x) override { this->parent_->start(); }
};

template<typename... Ts> class StopAction : public Action<Ts...>, public Parented<AlkoveSpot> {
 public:
  void play(const Ts &...x) override { this->parent_->stop(); }
};

template<typename... Ts> class IsRunningCondition : public Condition<Ts...>, public Parented<AlkoveSpot> {
 public:
  bool check(const Ts &...x) override { return this->parent_->is_running(); }
};

}  // namespace alkove_spot
}  // namespace esphome
