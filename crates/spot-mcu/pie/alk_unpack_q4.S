// alk_unpack_q4(codes, zero, m, out, blocks): 4-bit blocks of 32 to int8 with PIE, as alkove-spot-mcu's unpack_q4:
// w8 = ((code - zero) * m + 8) >> 4, codes low nibble first; codes and out 16-byte aligned; |code - zero| * m <= 2031, so no clamp.
// a2 codes, a3 zero, a4 m, a5 out, a6 blocks. q7 0x0f bytes, q6 8 per s16 lane, q5 1 per s16 lane, q4 zero, q3 m.

    .section .rodata
    .align 16
alk_q4_consts:
    .fill 16, 1, 0x0f
    .fill 8, 2, 8
    .fill 8, 2, 1

    .text
    .align 4
    .global alk_unpack_q4
    .type alk_unpack_q4, @function
alk_unpack_q4:
    entry a1, 32
    beqz a6, .Ldone
    movi a8, alk_q4_consts
    ee.vld.128.ip q7, a8, 16
    ee.vld.128.ip q6, a8, 16
    ee.vld.128.ip q5, a8, 16
    movi a9, 4
    movi a10, 0
.Lblock:
    ee.vldbc.8 q4, a3
    ee.zero.q q0
    ee.vzip.8 q4, q0
    ee.vldbc.8 q3, a4
    ee.zero.q q0
    ee.vzip.8 q3, q0
    addi a3, a3, 1
    addi a4, a4, 1

    ee.vld.128.ip q0, a2, 16
    wsr.sar a9
    ee.vsr.32 q1, q0
    ee.andq q1, q1, q7
    ee.andq q0, q0, q7
    ee.vzip.8 q0, q1

    // weights 0..15
    ee.zero.q q2
    ee.vzip.8 q0, q2
    ee.vsubs.s16 q0, q0, q4
    ee.vsubs.s16 q2, q2, q4
    wsr.sar a10
    ee.vmul.s16 q0, q0, q3
    ee.vmul.s16 q2, q2, q3
    ee.vadds.s16 q0, q0, q6
    ee.vadds.s16 q2, q2, q6
    wsr.sar a9
    ee.vmul.s16 q0, q0, q5
    ee.vmul.s16 q2, q2, q5
    ee.vunzip.8 q0, q2
    ee.vst.128.ip q0, a5, 16

    // weights 16..31
    ee.zero.q q2
    ee.vzip.8 q1, q2
    ee.vsubs.s16 q1, q1, q4
    ee.vsubs.s16 q2, q2, q4
    wsr.sar a10
    ee.vmul.s16 q1, q1, q3
    ee.vmul.s16 q2, q2, q3
    ee.vadds.s16 q1, q1, q6
    ee.vadds.s16 q2, q2, q6
    wsr.sar a9
    ee.vmul.s16 q1, q1, q5
    ee.vmul.s16 q2, q2, q5
    ee.vunzip.8 q1, q2
    ee.vst.128.ip q1, a5, 16

    addi a6, a6, -1
    bnez a6, .Lblock
.Ldone:
    retw.n
    .size alk_unpack_q4, . - alk_unpack_q4
