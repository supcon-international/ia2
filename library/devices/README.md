# Device catalog（设备模板目录）

一目录一文件：经过验证的现场设备模板。`/api/devices` 创建流程与
EtherCAT 总线发现（`/discover`）用它把"手填 channel"变成"识别 →
预填 → 微调"：发现的 `vendor_id`/`product_id` 命中模板时，设备的
slaves / channels / DC 要求直接从模板生成，不再对着日志抄字节偏移。

## 条目格式

与项目 `devices/*.toml` 同构，去掉现场专属字段（`nic`），加上元数据：

```toml
name = "InoSV660N"             # 默认设备名（可改）
protocol = "ethercat"
description = "…"              # 一行人话
vendor_id = 1048576             # 识别键（EtherCAT ESI identity）
product_id = 786701
requires_dc_sync = "sync0"     # 不满足直接到不了 OP 的硬约束
recommended_cycle_us = 2000

[[channels]]                    # PDO/寄存器模板，实测过的布局
…
```

规则：

- **只收实测过的模板**——在真实硬件上跑通（连接 + OP + 数据正确）
  才能入目录；来源（哪台台架、哪个项目）写进文件头注释。
- 不放客户/项目信息；设备厂商与公开型号属于硬件事实，允许出现。
- Modbus 设备同理（识别靠人选型号，无总线发现）：寄存器表 + 字序
  + 推荐 poll 周期。
