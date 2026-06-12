(*
  FB_STATS — 累计统计：最小/最大/均值/标准差（Welford 单遍递推）。

  ≈ 标准统计块（min/max/average/standard deviation），各主流平台
  标准库均有同类块；本块用 Welford 单遍递推，无数组、数值稳定。

  用途：质量数据统计（批次温度均值/波动）、测量噪声水平评估、
  调试期信号摸底（接 FB_AVG_TIME 看时间平均，接本块看全程分布）。
  行为（按"样本"计，不吃 dt_s：每个 enable 为 TRUE 的扫描拍计 1 个样本）：
    Welford 递推（标量，单遍，避免大数平方相减的精度灾难）：
      n := n + 1；delta := u - avg；avg := avg + delta / n；
      m2 := m2 + delta * (u - avg)
    sd := SQRT(m2 / n) —— 总体标准差（除 n 而非 n-1）：过程监视看的是
    波动幅度，样本数大时两者无差，且单遍递推下除 n 形式最直接。
    reset 电平有效：清零并在下一个使能样本重新播种（vmin/vmax/avg 取
    该样本值）。
    注意：n 为 REAL 计数，单精度下计到约 1.6E7 个样本（100 ms 采样约
    19 天）后 +1 失效、统计冻结——长期运行请按班/按天 reset。

  输入：
    u      REAL  输入样本
    enable BOOL  采样使能（FALSE 拍不计样本，输出保持）
    reset  BOOL  清零（电平有效，优先；下一使能样本重新播种）
  输出：
    vmin REAL  最小值
    vmax REAL  最大值
    avg  REAL  算术均值
    sd   REAL  总体标准差
    n    REAL  已计样本数
*)
FUNCTION_BLOCK FB_STATS
  VAR_INPUT
    u      : REAL;
    enable : BOOL := TRUE;
    reset  : BOOL := FALSE;
  END_VAR
  VAR_OUTPUT
    vmin : REAL;
    vmax : REAL;
    avg  : REAL;
    sd   : REAL;
    n    : REAL;
  END_VAR
  VAR
    m2     : REAL; (* Welford 二阶累计量 Σ(u-avg)·(u-avg') *)
    delta  : REAL;
    delta2 : REAL;
  END_VAR

  IF reset THEN
    n := 0.0;
    avg := 0.0;
    m2 := 0.0;
    sd := 0.0;
    vmin := 0.0;
    vmax := 0.0;
    RETURN;
  END_IF;

  IF NOT enable THEN
    RETURN; (* 不计样本：输出保持 *)
  END_IF;

  IF n <= 0.0 THEN
    (* 首样本播种 *)
    n := 1.0;
    avg := u;
    m2 := 0.0;
    sd := 0.0;
    vmin := u;
    vmax := u;
    RETURN;
  END_IF;

  n := n + 1.0;
  delta := u - avg;
  avg := avg + delta / n;
  delta2 := u - avg;
  m2 := m2 + delta * delta2;
  sd := SQRT(m2 / n);

  IF u < vmin THEN
    vmin := u;
  END_IF;
  IF u > vmax THEN
    vmax := u;
  END_IF;
END_FUNCTION_BLOCK
