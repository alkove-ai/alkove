"""Alkove Spot: typed wake words for ESPHome, in place of micro_wake_word (alkove/integrations/esphome)."""
from pathlib import Path
import shutil
import wave

from esphome import automation
import esphome.codegen as cg
from esphome.components import esp32, microphone
import esphome.config_validation as cv
from esphome.const import CONF_ID, CONF_MICROPHONE, CONF_MODEL
from esphome.core import CORE

CODEOWNERS = ["@edbrdi"]
DEPENDENCIES = ["esp32", "psram"]
AUTO_LOAD = ["ring_buffer"]

CONF_LIBRARY = "library"
CONF_WORDS = "words"
CONF_NAME = "name"
CONF_PHONEMES = "phonemes"
CONF_IDS = "ids"
CONF_P = "p"
CONF_FRENCH = "french"
CONF_THRESHOLDS = "thresholds"
CONF_ANYWHERE = "anywhere"
CONF_PAUSE = "pause"
CONF_CONTEXT = "context"
CONF_STEPS_PER_CALL = "steps_per_call"
CONF_TASK_CORE = "task_core"
CONF_TEST_AUDIO = "test_audio"
CONF_ON_WAKE_WORD_DETECTED = "on_wake_word_detected"

alkove_spot_ns = cg.esphome_ns.namespace("alkove_spot")
AlkoveSpot = alkove_spot_ns.class_("AlkoveSpot", cg.Component)
StartAction = alkove_spot_ns.class_("StartAction", automation.Action)
StopAction = alkove_spot_ns.class_("StopAction", automation.Action)
IsRunningCondition = alkove_spot_ns.class_("IsRunningCondition", automation.Condition)

CONTEXTS = {"pause": 0, "anywhere": 1, "both": 2}

VARIANT_SCHEMA = cv.Schema(
    {
        cv.Required(CONF_IDS): cv.All(cv.ensure_list(cv.int_range(1, 58)), cv.Length(min=1)),
        cv.Optional(CONF_P, default=1.0): cv.float_range(0, 1),
        cv.Optional(CONF_FRENCH, default=False): cv.boolean,
    }
)

WORD_SCHEMA = cv.Schema(
    {
        cv.Required(CONF_NAME): cv.string_strict,
        cv.Required(CONF_PHONEMES): cv.All(cv.ensure_list(VARIANT_SCHEMA), cv.Length(min=1)),
        cv.Required(CONF_THRESHOLDS): cv.Schema({cv.Required(CONF_ANYWHERE): cv.float_, cv.Required(CONF_PAUSE): cv.float_}),
    }
)

CONFIG_SCHEMA = cv.All(
    cv.Schema(
        {
            cv.GenerateID(): cv.declare_id(AlkoveSpot),
            cv.Optional(CONF_MICROPHONE): microphone.microphone_source_schema(min_bits_per_sample=16, max_bits_per_sample=16, min_channels=1, max_channels=1),
            cv.Required(CONF_MODEL): cv.file_,
            cv.Required(CONF_LIBRARY): cv.file_,
            cv.Required(CONF_WORDS): cv.All(cv.ensure_list(WORD_SCHEMA), cv.Length(min=1)),
            cv.Optional(CONF_CONTEXT, default="pause"): cv.one_of(*CONTEXTS, lower=True),
            cv.Optional(CONF_STEPS_PER_CALL, default=8): cv.int_range(1, 8),
            # ESPHome's loop runs on core 1
            cv.Optional(CONF_TASK_CORE, default=0): cv.int_range(0, 1),
            cv.Optional(CONF_TEST_AUDIO): cv.file_,
            cv.Optional(CONF_ON_WAKE_WORD_DETECTED): automation.validate_automation(single=True),
        }
    ).extend(cv.COMPONENT_SCHEMA),
    cv.only_on_esp32,
)


def FINAL_VALIDATE_SCHEMA(config):
    if CONF_MICROPHONE in config:
        microphone.final_validate_microphone_source_schema("alkove_spot", sample_rate=16000)(config[CONF_MICROPHONE])
    return config


def _test_pcm(path: Path) -> bytes:
    with wave.open(str(path), "rb") as w:
        if w.getnchannels() != 1 or w.getsampwidth() != 2 or w.getframerate() != 16000:
            raise cv.Invalid(f"{path}: test_audio must be 16 kHz mono 16-bit WAV")
        return w.readframes(w.getnframes())


async def to_code(config):
    # the ESP-IDF component: the Rust library, its header, the model and the test audio, built from idf/
    here = Path(__file__).parent
    idf = Path(CORE.relative_build_path("alkove_spot_idf"))
    shutil.rmtree(idf, ignore_errors=True)
    shutil.copytree(here / "idf", idf)
    (idf / "lib").mkdir()
    (idf / "include").mkdir()
    (idf / "data").mkdir()
    shutil.copy(config[CONF_LIBRARY], idf / "lib" / "libalkove_spot_esp.a")
    shutil.copy(here.parent.parent / "lib" / "include" / "alk_spot.h", idf / "include")
    shutil.copy(config[CONF_MODEL], idf / "data" / "model.mcu")
    (idf / "data" / "test.pcm").write_bytes(_test_pcm(Path(config[CONF_TEST_AUDIO])) if CONF_TEST_AUDIO in config else b"")
    esp32.add_idf_component(name="alkove_spot_idf", path=str(idf))
    esp32.add_idf_component(name="espressif/esp-nn", ref="1.4.1")

    var = cg.new_Pvariable(config[CONF_ID])
    await cg.register_component(var, config)

    if CONF_MICROPHONE in config:
        cg.add_define("USE_ALKOVE_SPOT_MICROPHONE")
        mic_source = await microphone.microphone_source_to_code(config[CONF_MICROPHONE])
        cg.add(var.set_microphone_source(mic_source))

    for word in config[CONF_WORDS]:
        th = word[CONF_THRESHOLDS]
        for v in word[CONF_PHONEMES]:
            cg.add(var.add_variant(word[CONF_NAME], cg.std_vector.template(cg.uint32)(v[CONF_IDS]), v[CONF_P], v[CONF_FRENCH], th[CONF_ANYWHERE], th[CONF_PAUSE]))
    cg.add(var.set_context(CONTEXTS[config[CONF_CONTEXT]]))
    cg.add(var.set_steps_per_call(config[CONF_STEPS_PER_CALL]))
    cg.add(var.set_task_core(config[CONF_TASK_CORE]))
    cg.add(var.set_self_test(CONF_TEST_AUDIO in config))

    if on_wake := config.get(CONF_ON_WAKE_WORD_DETECTED):
        await automation.build_automation(var.get_wake_word_detected_trigger(), [(cg.std_string, "wake_word")], on_wake)


SPOT_ACTION_SCHEMA = cv.Schema({cv.GenerateID(): cv.use_id(AlkoveSpot)})


@automation.register_action("alkove_spot.start", StartAction, SPOT_ACTION_SCHEMA)
@automation.register_action("alkove_spot.stop", StopAction, SPOT_ACTION_SCHEMA)
async def spot_action_to_code(config, action_id, template_arg, args):
    var = cg.new_Pvariable(action_id, template_arg)
    await cg.register_parented(var, config[CONF_ID])
    return var


@automation.register_condition("alkove_spot.is_running", IsRunningCondition, SPOT_ACTION_SCHEMA)
async def spot_running_to_code(config, condition_id, template_arg, args):
    var = cg.new_Pvariable(condition_id, template_arg)
    await cg.register_parented(var, config[CONF_ID])
    return var
