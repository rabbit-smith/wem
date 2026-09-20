# 2ch/48 kHz residue：aoTuV beta6.03 对照

日期：2026-09-20

## 结论

Wwise 2013.2 的双声道 residue handoff 不是仓库原先实现的
`round(mdct / floor)` 加可逆 mapping coupling，也不是 stock libvorbis
1.3.3 的充分描述。Wwise 运行时调用的函数与 aoTuV beta6.03 的
`_vp_couple_quantize_normalize` 一致。

决定性证据有三项：

1. Audiokinetic 的 Wwise 2011.2.2 发布说明明确记录 Vorbis encoder 更新到
   aoTuV beta6.03；2013.2 的 the paired build 仍呈现该函数的参数、结构字段和
   分支常量。
2. 动态观测得到短块 `n=128, normal_partition=8`，长块
   `n=1024, normal_partition=32`；point limit 分别为 42/341，编码低通为
   96/768，pre/post point threshold 为 0/2.5，rephase threshold 为 0/0.5。
3. 将同一次调用的 raw MDCT、整数 floor 曲线、impulse peak 和 nonzero 输入
   送入公开 aoTuV 算法后，两批样本的 217,088 个输出整数与 Wwise 返回值
   逐系数相同，差异为 0。样本同时覆盖 short 和 long。

这证明分类和 VQ 之前必须执行 aoTuV 的联合 quantize/coupling。反汇编地址只用于
提出候选；最终语义由函数输入输出观测和公开源码共同确认。

## 一手资料

- [aoTuV beta6.03 官方源码](https://ao-yumi.github.io/aotuv_web/source_code/libvorbis-aotuv_b6.03.tar.bz2)，重点是
  `lib/psy.c` 的 `flag_lossless`、`noise_normalize` 和
  `_vp_couple_quantize_normalize`。
- [Wwise 2011.2.2 Release Notes](https://www.audiokinetic.com/download/documents/Wwise_v2011.2.2_ReleaseNotes.pdf)，记录 encoder 更新到 aoTuV beta6.03。
- [Vorbis I specification](https://xiph.org/vorbis/doc/Vorbis_I_spec.html)，用于确认 mapping nonzero propagation、type-2 flat 域和 decoder 顺序。
- [Xiph libvorbis v1.3.3](https://github.com/xiph/vorbis/tree/v1.3.3)，用于核对通用 type-2/classword/VQ 结构；它不代替 aoTuV 的 encoder 语义。

## 已确认的 handoff 算法

每个 normal partition 执行以下步骤：

1. floor1 编码器先生成整数 0..255 floor 曲线；量化器再通过
   `FLOOR1_fromdB_LOOKUP` 取幅度。
2. `flag_lossless` 计算 `mdct / floor`，结合 coupling point limit、
   impulse peak、point threshold 和 rephase threshold 选择 lossless、point 或
   rephase 候选。
3. 每通道先做整数化/noise normalization。本 profile 的
   `normal_start=9999`，观测区间内退化为 nearest-even `rint`，但算法的分块和
   后续 coupling 仍然有效。
4. M6 按整块统计相位反转比例和双声道 residue 偏差；它可能把当前 partition
   的 rephase 候选提升为 lossless。
5. lossless 分支同时耦合浮点 residue 和已整数化 residue；point 分支使用
   aoTuV `min_indemnity_dipole_hypot`，angle 置零，再量化 magnitude。
6. lowpass 之后的系数清零，coupling pair 的 nonzero 状态传播到两行。

短块的 `normal_partition=8` 是关键。曾经把 32 写死后，长块可以完全匹配，短块
仍有差异；改为运行时观测值 8 后，short/long 都逐系数匹配。

## 独立修复项

type-2 只在所有通道都 unused 时跳过。coupling pair 任一 floor used 时，必须把
nonzero 传播到两行，并保持 `[M0,A0,M1,A1,...]` 的固定 flat stride。Python 和
Rust 现在都显式传播该状态，并有一边 floor unused 的回归测试。

classword setup 覆盖完整的 `10²` 组合。旧代码遇到未编码 classword 时静默换成
最小可用 entry 会改变分类结果，现已改成显式错误。

## 被排除或撤回的路线

- 仅尝试 decoder 四分支 coupling 的可逆前像，无法生成 aoTuV 的 forward
  handoff，因此不是修复。
- stock Xiph 1.3.3 只能解释通用 Vorbis 结构，不能证明 Wwise 的 encoder 分支。
- 旧手写 reader 的 38×/3× 非零倍率来自失步解析，已经撤回；可靠 decoder 的
  classword、stage、book 和 VQ 统计见
  [`2ch-residue-comparison.md`](2ch-residue-comparison.md)。
- floor 曲线两侧在观测样本中接近，不等于 floor 与 residue 联合量化可以分开；
  aoTuV handoff 使用整数 floor 曲线和额外 peak surface。

## 端到端闭合

量化函数在真实 Wwise 输入上完全相同，直接证明的是 handoff 语义。最终 WEM 还取决于
上游 MDCT、floor fit、impulse peak、frame plan 以及下游 class/VQ 打包；这些边界随后
通过完整 WEM、逐包和仓库 decoder 的独立观测闭合。当前结果和证据范围见
[`2ch-active-findings.md`](2ch-active-findings.md)。局部规则仍必须锚定独立可观测量，不能用
总字节数反推。
