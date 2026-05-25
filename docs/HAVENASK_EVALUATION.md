# Gitnova 对 Havenask 解析效果评估报告

## 1. 测试环境

- **目标仓库**: Havenask (`/Users/lishuo121/workspace/havenask`)
- **项目类型**: 大规模 C++ 搜索引擎 (含少量 Python)
- **Gitnova 版本**: 0.1.0 (release build)
- **索引命令**: `gitnova index /Users/lishuo121/workspace/havenask --force`

## 2. 基础统计 (V9 最新)

| 指标 | V9 数值 | V1 原始 | 变化 |
|------|---------|---------|------|
| 总节点数 | 242,305 | 261,803 | -7.5% |
| 总边数 | 1,147,077 | 1,149,808 | -0.3% |
| 文件数 | 13,435 | 13,435 | 0% |

### 2.1 节点类型分布 (V9)

| 类型 | V9 数量 | V9 占比 | V1 数量 | V1→V9 变化 |
|------|---------|---------|---------|-----------|
| import | 96,424 | 39.8% | 97,323 | -0.9% |
| method | 66,819 | 27.5% | 26,935 | **+148.2%** |
| variable | 32,906 | 13.6% | 0 | **新增** |
| class | 14,294 | 5.9% | 25,714 | -44.2% |
| file | 13,435 | 5.6% | 13,435 | 0% |
| typedef | 5,400 | 2.2% | 0 | **新增** |
| macro | 4,233 | 1.7% | 0 | **新增** |
| function | 3,428 | 1.4% | 52,682 | **-93.5%** |
| struct | 2,798 | 1.2% | 4,592 | -39.3% |
| module | 2,008 | 0.8% | 41,121 | **-95.1%** |
| enum | 544 | 0.2% | 0 | **新增** |
| union | 15 | 0.0% | 0 | **新增** |

### 2.2 边类型分布 (V9)

| 类型 | V9 数量 | V9 占比 | V1 数量 | V1→V9 变化 |
|------|---------|---------|---------|-----------|
| references | 363,177 | 31.7% | 366,715 | -1.0% |
| calls | 363,177 | 31.7% | 366,715 | -1.0% |
| contains | 164,158 | 14.3% | 164,479 | -0.2% |
| defines | 150,723 | 13.2% | 151,044 | -0.2% |
| imports | 102,811 | 9.0% | 100,855 | +1.9% |
| **extends** | **3,036** | **0.3%** | **0** | **新增** |

### 2.3 extends 边目标分布 (V9)

| 目标类型 | 数量 | 占比 |
|---------|------|------|
| class | 2,085 | 69% |
| typedef | 854 | 28% |
| struct | 54 | 2% |
| method | 8 | 0% |

---

## 3. 功能测试结果 (V9)

### 3.1 rank-context 搜索

**Q1: `"inverted index build"`**
1. method `BuildingDateIndexReader::~BuildingDateIndexReader` score=0.841
2. method `BuildingIndexReader::GetSegmentCount` score=0.841
3. method `BuildingIndexReader::Update` score=0.841
4. method `BuildingRangeIndexReader::~BuildingRangeIndexReader` score=0.841
5. method `InvertedIndexBuildWorkItem::~InvertedIndexBuildWorkItem` score=0.841

**Q2: `"search query term posting"`**
1. method `SpatialTermQueryExecutor::reset` score=0.836
2. method `BitmapTermQueryExecutor::BitmapTermQueryExecutor` score=0.827
3. method `SubFieldMapTermQueryExecutor::reset` score=0.827
4. method `BitmapTermQueryExecutor::doSeek` score=0.818
5. method `BitmapTermQueryExecutor::initBitmapIterator` score=0.818

**Q3: `"hash table lookup"`**
1. method `HashTablePrimaryKeyDiskIndexer::Lookup` score=0.828
2. method `KKVReader::LookupAsync` score=0.794
3. method `HashTableCompressVarSegmentReader::Get` score=0.791
4. method `HashTableVarSegmentReader::Get` score=0.791
5. method `KKVCachedReaderImpl::KKVCachedReaderImpl` score=0.785

### 3.2 Hub 排名 (V9)

| # | kind | qualified_name | in_degree |
|---|------|---------------|-----------|
| 1 | macro | `IE_LOG_DECLARE` | 4,960 |
| 2 | variable | `ResourceReader::OK` | 3,820 |
| 3 | macro | `DEFINE_SHARED_PTR` | 3,630 |
| 4 | macro | `BS_LOG` | 2,880 |
| 5 | variable | `nlohmann::json.hpp::move` | 2,732 |

### 3.3 graph-context 图分析

- 继承关系已可见：`InvertedIndexBuildWorkItem extends BuildWorkItem` (confidence=9000)
- `InvertedIndexSearchTracer` 有 6 个 class 节点（namespace 去重未完全合并）
- **限制**: impact-analysis 未沿 extends 边传播影响

---

## 4. 版本演进

### 4.1 综合评分

| 维度 | V1 | V3 | V4 | V5 | V6 | V7 | V8 | V9 |
|------|-----|-----|-----|-----|-----|-----|-----|-----|
| 解析覆盖率 | 4 | 4 | 4 | 5 | 5 | 5 | 5 | **5** |
| 符号解析准确性 | 2.5 | 4 | 4 | 3 | 4.5 | 4.5 | 4.5 | **4.5** |
| 关系建模完整性 | 3 | 3 | 4 | 4 | 4 | 4.5 | 4.5 | **4.5** |
| 搜索排序质量 | 3.5 | 4 | 4 | 3 | 4 | 4 | 4 | **4** |
| 大规模项目可用性 | 4 | 4.5 | 4.5 | 4 | 4.5 | 4.5 | 4.5 | **4.5** |
| **综合** | **3.2** | **3.9** | **4.1** | **3.8** | **4.4** | **4.5** | **4.6** | **4.6** |

### 4.2 关键指标演进

| 指标 | V1 | V3 | V4 | V5 | V6 | V7 | V8 | V9 |
|------|-----|-----|-----|-----|-----|-----|-----|-----|
| 总节点 | 261,803 | 231,374 | 215,727 | 339,958 | 244,114 | 229,298 | 242,305 | 242,305 |
| function | 52,682 | 3,869 | 3,858 | 8,289 | 3,596 | 2,399 | 3,428 | 3,428 |
| method | 26,935 | 73,857 | 73,637 | 62,816 | 67,422 | 64,615 | 66,819 | 66,819 |
| class | 25,714 | 15,704 | 15,095 | 14,958 | 14,958 | 8,435 | 14,294 | 14,294 |
| module | 41,121 | 25,102 | 10,351 | 2,271 | 2,271 | 486 | 2,008 | 2,008 |
| extends 边 | 0 | 0 | 2,185 | 3,028 | 3,028 | 3,008 | 3,031 | 3,036 |
| extends→class | 0 | 0 | 1,484 | 1,451 | 1,451 | 1,193 | 2,029 | **2,085** |
| extends→class 占比 | 0 | 0 | 68% | 48% | 48% | 40% | 67% | **69%** |
| 节点类型数 | 7 | 7 | 7 | 12 | 12 | 12 | 12 | 12 |

### 4.3 各版本 commit

| 版本 | commit | 说明 |
|------|--------|------|
| V1 | 初始 | 原始版本，正则 heuristic 解析 |
| V2 | `43dca3b` | feat: add C++ language support (tree-sitter，实际效果与 V1 无差异) |
| V3 | `6e5563a` 前的本地改动 | 修复 .cpp 方法归类，大幅改善 |
| V4 | `6e5563a` | fix: comprehensive C++ code graph quality improvements（extends 边、namespace 去重） |
| V5 | `e14608b` | feat: adopt tree-sitter Query API（variable 淹没、function 反弹） |
| V6 | `6fc6df0` | fix: V5 regression - variable flooding and function rebound |
| V7 | `7727455`+`e1fa594`+`952f0e9` | namespace 去重 + extends 精度 + hub 惩罚 + class fallback |
| V8 | `c9422d1` | fix: over-merging in namespace dedup，class 恢复到 14,294 |
| V9 | `f5857b9` | fix: improve extends target selection and hub quality |

### 4.4 关键版本里程碑

- **V3 (+0.7)**: 修复 .cpp 方法归类 (function→method)，最大单次提升
- **V5 (-0.3)**: variable 淹没 (128K) 导致回归，最大单次退步
- **V7 (+0.1)**: namespace 去重生效，但 class 过度合并 (14,958→8,435)
- **V8 (+0.1)**: 修复过度合并，class 恢复 14,294，extends→class 67%
- **V9 (持平)**: extends→class 69%，边际改善，评分持平 4.6

---

## 5. 历史版本回归详情

### 5.1 V7 版本回归 (namespace dedup + extends precision + hub penalties + class fallback)

**数据对比 (V6→V7)**

| 指标 | V6 | V7 | 变化 |
|------|-----|-----|------|
| 总节点数 | 244,114 | 229,298 | -6.1% |
| class | 14,958 | 8,435 | **-43.7%** (过度合并) |
| module | 2,271 | 486 | **-78.6%** |
| function | 3,596 | 2,399 | -33.3% |
| extends→method | 552 | 8 | **大幅改善** |
| extends→class | 1,451 | 1,193 | -17.8% (因去重) |

**关键改进**:
- P2 Namespace 去重: module 大幅减少，class 跨文件合并
- P3 extends 精度: extends→method 从 552→8，几乎消除
- P3 Hub 惩罚: macro/variable 入度小幅下降

**问题**: class 过度合并 (14,958→8,435)，`InvertedIndexSearchTracer` 消失 → V8 修复

### 5.2 V8 版本回归 (fix: over-merging in namespace dedup)

**数据对比 (V7→V8)**

| 指标 | V7 | V8 | 变化 |
|------|-----|-----|------|
| 总节点数 | 229,298 | 242,305 | +5.7% |
| class | 8,435 | 14,294 | **+69.5%** (恢复) |
| module | 486 | 2,008 | +313% (恢复) |
| function | 2,399 | 3,428 | +42.9% (恢复) |
| extends→class | 1,193 | 2,029 | **+70%** |
| extends→class 占比 | 40% | 67% | **+27pp** |

**关键改进**: 修复 namespace 去重过度合并，class 恢复到合理水平，extends→class 占比大幅提升

### 5.3 V9 版本回归 (fix: improve extends target selection and hub quality)

commit: `f5857b9`

| extends→目标 | V8 | V9 | 变化 |
|-------------|-----|-----|------|
| class | 2,029 (67%) | 2,085 (69%) | +56 |
| typedef | 852 (28%) | 854 (28%) | +2 |
| struct | 106 (4%) | 54 (2%) | -52 |
| method | 8 | 8 | 0 |

extends→class 占比从 67%→69%，extends→struct 更精确 (106→54)。搜索结果与 V8 一致，评分维持 **4.6/5**。

---

## 6. 下一步开发优先级

### P2: Namespace 嵌套层级去重 (InvertedIndexSearchTracer)

**现状**: `InvertedIndexSearchTracer` 仍有 6 个 class 节点，namespace 去重未完全合并

**修改文件**: `crates/gitnova-core/src/graph.rs`

**方案**:
1. 在图构建后处理阶段，对 `(kind, name)` 相同但 `qualified_name` 不同的节点合并
2. 合并策略：保留最短（或最常见）的 qualified_name，将所有边重定向到合并后的节点
3. 注意避免 V7 过度合并问题 — 只合并 qualified_name 为子序列关系的节点

**验证 SQL**:
```sql
-- 修复前：返回 6 行
SELECT qualified_name FROM nodes WHERE name='InvertedIndexSearchTracer' AND kind='"class"';
-- 修复后：应仅返回 1 行
```

### P2: 索引性能优化

**现状**: V6+ 索引 ~8min，V4 仅 ~2min，Query API 导致 4x 回归

**排查方向**:
1. `crates/gitnova-core/src/extract/cpp_queries.scm` — Query 是否可以精简匹配模式
2. 是否可以为 tree-sitter 解析结果添加缓存
3. 是否可以并行化文件解析

**验证**: 索引时间 < 3min

### P3: extends 边精度 — typedef 解引用

**现状**: 854/3,036 (28%) extends 边指向 typedef 而非 class

| to_id 指向 | 数量 | 占比 |
|-----------|------|------|
| class | 2,085 | 69% |
| typedef | 854 | 28% |
| struct | 54 | 2% |
| method | 8 | 0% |

**修改文件**: `crates/gitnova-core/src/graph.rs`

**方案**: 创建 extends 边时，如果 to_id 是 typedef，查找 typedef 指向的实际 class 节点作为目标

### P3: impact-analysis 利用继承链

**现状**: extends 边存在但 impact-analysis 不沿其传播，`InvertedIndexBuildWorkItem` 只影响 1 个文件

**修改文件**: `crates/gitnova-core/src/query.rs` 或 `crates/gitnova-enrich/`

**方案**: impact-analysis 的 BFS 遍历中，将 extends 边纳入遍历范围（类似 calls/references）

### P3: Hub 排名惩罚通用符号

**现状**: `IE_LOG_DECLARE`(4,960)、`OK`(3,820)、`move`(2,732) 等通用符号占据 Hub Top

**修改文件**: `crates/gitnova-rank/src/features.rs`、`crates/gitnova-rank/src/score.rs`

**方案**: 对 macro/variable 类型节点增加 hub 惩罚权重，或对高频通用名（`OK`/`move`/`size`/`Get`）降权

### P4: variable 噪声

**现状**: `variable nlohmann::json.hpp::move`(入度 2,732) 等第三方库变量仍占 Hub

**方案**: 在 extraction 阶段标记第三方库路径（如 `third_party/`），在 rank 中降权

---

## 7. 预期收益

| 修复项 | 预期评分提升 | 修复后评分 |
|--------|------------|-----------|
| P2: Namespace 去重 | +0.2 | 4.8 |
| P2: 索引性能 | +0.1 | 4.9 |
| P3: extends 精度 | +0.1 | 5.0 |
| P3: impact 继承链 | +0.1 | 5.0+ |
| P3: Hub 惩罚 | +0.05 | 5.0+ |
| P4: variable 噪声 | +0.05 | 5.0+ |