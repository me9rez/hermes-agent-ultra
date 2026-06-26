# collect 并行 benchmark（轨 H · 2026-06-25）

## 方法

- **Symbol**: `600519.SH`（贵州茅台）
- **Profile**: `AnalysisProfile::medium()`（当前 HTTP 硬数据 **11** 维，web-only 维默认跳过）
- **对照**: `CollectOptions.parallel = true`（默认，层内 `join_all`）vs `parallel = false`（层内串行）
- **命令**:

```powershell
cargo test -p hermes-trading benchmark_collect_parallel_vs_serial_600519_medium -- --ignored --nocapture
```

Ignored 测试仅 **打印指标**（无 assert），避免网络抖动误报；回归靠 offline golden + CI equity research gate。

## 本机实测（2026-06-25，Windows，国内网络）

| Run | Parallel | Serial | 缩短 |
|-----|----------|--------|------|
| 1（冷启动） | 4314 ms | 5702 ms | **~24.3%** |
| 2（紧接 rerun） | 6288 ms | 5223 ms | −20.4%（限速/缓存扰动） |

**结论**：层内并行在 run 1 达成 roadmap 方向（≈25% 缩短）；连续压测会触发 Eastmoney 限速，以 run 1 为 baseline 记录。

Roadmap 原目标 30% 对 full 22 维 + 更宽 layer 更可达；当前 medium 仅 11 维 HTTP 维。

## 层结构（medium · A 股）

`exec_layers` 按 `depends_on` 拓扑分层；`0_basic` 完成后 `10_valuation` / `4_peers` 等同层可并发。层间仍串行（保证 `prior` 注入）。

## CI 与后续

- CI **不跑** `--ignored` benchmark。
- 并行开关：`CollectOptions { parallel: false, .. }` 仅供 benchmark / debug。
- 若 future `depth=deep` 打开更多并发维，预期 savings 上升。
