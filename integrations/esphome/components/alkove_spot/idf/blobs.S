// the model and the self-test audio, 16-byte aligned so Spot reads its weights in place; copied here by __init__.py

    .section .rodata.alkove_spot_model, "a"
    .balign 16
    .global alkove_spot_model
alkove_spot_model:
    .incbin "model.mcu"
    .global alkove_spot_model_end
alkove_spot_model_end:

    .section .rodata.alkove_spot_test, "a"
    .balign 16
    .global alkove_spot_test
alkove_spot_test:
    .incbin "test.pcm"
    .global alkove_spot_test_end
alkove_spot_test_end:
