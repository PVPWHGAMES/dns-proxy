# Baseline Governance

## 1. Baseline Roles
- Product / Requirement Baseline：已确认的需求、目标、场景、验收标准和非目标。
- Architecture / Runtime Boundary Baseline：窗口生命周期、配置源和托盘恢复行为的 canonical owner。

## 2. Design Defect
已确认的需求、设计或基线中的错误、缺口或矛盾应先修正基线，再调整实现。

## 3. Implementation Drift
实现偏离正确且未改变的需求或架构基线时，应回到基线，以最简单稳定的方式修复。

## 4. Compatibility Aliases
- Architecture Defect = architecture-scoped Design Defect
- Architecture Drift = architecture-scoped Implementation Drift

## 5. Baseline Check Protocol
非平凡修改前读取最新产品/需求基线和架构/运行边界基线，并检查验收标准、owner、contract 和兼容边界。

## 6. Architecture Review — 7 Dimensions
1. ownership integrity
2. module boundaries
3. contract changes
4. cascade proliferation
5. dependency direction
6. retirement completeness
7. entropy flow

## 7. Hard Boundaries
- 本文件是本项目 Aegis 工作区的治理约束。
- 基线快照是证据，不替代已确认需求。
- ADR 记录决策，不替代基线治理。
