#![allow(unused_must_use)]

use std::env;
use std::path::PathBuf;

use gitnova_wiki::{ContentFormat, Evidence, PageKind, PatchMode, WikiPage, WikiStore};

fn main() -> anyhow::Result<()> {
    let out_dir = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/Users/lishuo121/workspace/havenask/.gitnova/wiki"));
    if out_dir.exists() {
        std::fs::remove_dir_all(&out_dir)?;
    }
    let s = WikiStore::open(&out_dir)?;

    // ════════════════════════════════════════════════════════════════
    // Level 0: Root
    // ════════════════════════════════════════════════════════════════
    seed_page(&s, "havenask", "Havenask", PageKind::Index,
        "Large-scale distributed C++ search engine: indexing, storage, query, SQL, build, scheduling, and serving.",
        r##"## Architecture Overview

Havenask is a C++ search platform from Alibaba. The major subsystems are:

- **HA3** — query-facing search layer (parsing, execution, ranking)
- **Storage (indexlib)** — index storage, segment management, KV/KKV, inverted index
- **SQL** — SQL-over-search via Iquan optimizer and Navi graph execution
- **Navi** — DAG graph execution engine
- **Suez** — service framework (deploy, heartbeat, table management)
- **Build Service** — distributed index build and administration
- **Hippo** — resource scheduler
- **Catalog** — table/config metadata service
- **MatchDoc** — document reference and allocation framework
- **Expression** — expression evaluation and function registry
- **Protocol** — RPC protocol definitions
- **Plugins** — analyzer, UDF, and scorer plugins

```svg
<svg viewBox="0 0 920 320" role="img" aria-label="Havenask architecture overview" xmlns="http://www.w3.org/2000/svg">
  <defs><marker id="arrow" markerWidth="10" markerHeight="10" refX="8" refY="3" orient="auto" markerUnits="strokeWidth"><path d="M0,0 L0,6 L9,3 z" fill="#0f766e"/></marker></defs>
  <rect x="20" y="30" width="160" height="70" rx="8" fill="#e7f3f1" stroke="#0f766e"/>
  <text x="100" y="58" text-anchor="middle" font-size="14" font-weight="700" fill="#172033">SQL / Iquan</text>
  <text x="100" y="78" text-anchor="middle" font-size="11" fill="#667085">query planning</text>
  <rect x="220" y="30" width="160" height="70" rx="8" fill="#e7f3f1" stroke="#0f766e"/>
  <text x="300" y="58" text-anchor="middle" font-size="14" font-weight="700" fill="#172033">HA3 Search</text>
  <text x="300" y="78" text-anchor="middle" font-size="11" fill="#667085">parse → execute → rank</text>
  <rect x="420" y="30" width="160" height="70" rx="8" fill="#fff" stroke="#d7dce5"/>
  <text x="500" y="58" text-anchor="middle" font-size="14" font-weight="700" fill="#172033">Navi</text>
  <text x="500" y="78" text-anchor="middle" font-size="11" fill="#667085">DAG execution</text>
  <rect x="620" y="30" width="160" height="70" rx="8" fill="#fff" stroke="#d7dce5"/>
  <text x="700" y="58" text-anchor="middle" font-size="14" font-weight="700" fill="#172033">Suez</text>
  <text x="700" y="78" text-anchor="middle" font-size="11" fill="#667085">service framework</text>
  <rect x="20" y="150" width="240" height="70" rx="8" fill="#fff" stroke="#d7dce5"/>
  <text x="140" y="178" text-anchor="middle" font-size="14" font-weight="700" fill="#172033">Storage / indexlib</text>
  <text x="140" y="198" text-anchor="middle" font-size="11" fill="#667085">inverted / KV / KKV / segment</text>
  <rect x="300" y="150" width="200" height="70" rx="8" fill="#fff" stroke="#d7dce5"/>
  <text x="400" y="178" text-anchor="middle" font-size="14" font-weight="700" fill="#172033">Build Service</text>
  <text x="400" y="198" text-anchor="middle" font-size="11" fill="#667085">distributed indexing</text>
  <rect x="540" y="150" width="140" height="70" rx="8" fill="#fff" stroke="#d7dce5"/>
  <text x="610" y="178" text-anchor="middle" font-size="14" font-weight="700" fill="#172033">Hippo</text>
  <text x="610" y="198" text-anchor="middle" font-size="11" fill="#667085">scheduler</text>
  <rect x="720" y="150" width="180" height="70" rx="8" fill="#fff" stroke="#d7dce5"/>
  <text x="810" y="178" text-anchor="middle" font-size="14" font-weight="700" fill="#172033">Catalog</text>
  <text x="810" y="198" text-anchor="middle" font-size="11" fill="#667085">metadata service</text>
  <rect x="20" y="260" width="120" height="44" rx="8" fill="#f7f8fb" stroke="#d7dce5"/>
  <text x="80" y="287" text-anchor="middle" font-size="13" fill="#172033">MatchDoc</text>
  <rect x="160" y="260" width="130" height="44" rx="8" fill="#f7f8fb" stroke="#d7dce5"/>
  <text x="225" y="287" text-anchor="middle" font-size="13" fill="#172033">Expression</text>
  <rect x="310" y="260" width="120" height="44" rx="8" fill="#f7f8fb" stroke="#d7dce5"/>
  <text x="370" y="287" text-anchor="middle" font-size="13" fill="#172033">Protocol</text>
  <rect x="450" y="260" width="120" height="44" rx="8" fill="#f7f8fb" stroke="#d7dce5"/>
  <text x="510" y="287" text-anchor="middle" font-size="13" fill="#172033">Plugins</text>
  <path d="M100 100 V140" stroke="#0f766e" stroke-width="2" marker-end="url(#arrow)"/>
  <path d="M300 100 V140" stroke="#0f766e" stroke-width="2" marker-end="url(#arrow)"/>
  <path d="M500 100 V140" stroke="#667085" stroke-width="1.5" stroke-dasharray="4 4"/>
  <path d="M400 220 V255" stroke="#667085" stroke-width="1.5" stroke-dasharray="4 4"/>
</svg>
```

## Cross-Subsystem Data Flow

A search request flows through the system as follows:

1. **Client** sends SQL or HA3 query via ARPC
2. **QRS** (Query Result Server) receives the request
3. **SQL/Iquan** parses and optimizes into a Navi graph (if SQL)
4. **Navi** executes the DAG, dispatching to **HA3 Searcher** kernels
5. **HA3 Searcher** performs: QueryParser → Common Query → QueryExecutorCreator → QueryExecutor → SingleLayerSearcher → Filter → Rank
6. **indexlib** provides IndexPartitionReader / TabletReader for posting and attribute access
7. **MatchDoc** carries matched document references through the pipeline
8. **Rank** orders results via ComboComparator and MatchDocPriorityQueue
9. **QRS** aggregates and returns results to client

## Reading Path
Start from HA3 Search to understand the query path, then explore Storage/indexlib for index internals, SQL for declarative access, and Build Service for offline indexing."##,
        None, &[]);

    // ════════════════════════════════════════════════════════════════
    // Level 1: Storage / indexlib
    // ════════════════════════════════════════════════════════════════
    seed_page(&s, "havenask/storage", "Storage (indexlib)", PageKind::Index,
        "indexlib is the index storage layer: inverted index, attribute, summary, KV/KKV, segment management, and tablet framework.",
        r"## Core Modules
- **Inverted Index** — posting list read/write, term dictionary, bitmap index
- **Attribute** — columnar attribute storage, patch, expression, format
- **Summary** — document summary storage
- **KV / KKV** — primary-key store (single-value and multi-value)
- **Common** — hash table, data structure, block array, numeric compress, field format
- **Document** — raw document parsing, KV/KKV document, rewriter
- **Partition** — online partition, segment dump, operation queue
- **Tablet** — next-gen table abstraction (ITablet, TabletWriter, TabletReader)
- **File System** — package file, archive, flush, load config, relocatable
- **Framework** — lifecycle, cleaner, index task, memory reclaimer
- **Config** — index config, schema, field type definitions
- **Util** — cache, counter, memory control, slice array, buffer compressor

## Directory Structure
`aios/storage/indexlib/`:
- `index/inverted_index/` — PostingWriter, PostingIterator, InvertedIndexReader, SegmentPosting, DictionaryWriter
- `index/attribute/` — AttributeReader, AttributeWriter, AttributePatch, AttributeMerger, AttributeConfig
- `index/kkv/` — KKVSegmentWriter, KKVReader, ClosedHashPrefixKeyTable
- `index/common/hash_table/` — DenseHashTable, CuckooHashTable, SeparateChainHashTable
- `index/common/` — data_structure, block_array, numeric_compress, field_format, patch
- `document/` — RawDocument, NormalDocument, DocumentRewriter, FbKvRawDocumentParser
- `indexlib/partition/` — OnlinePartition, PartitionSegmentIterator, AsyncSegmentDumper
- `framework/` — ITablet, LifecycleTablet, IndexTask, Cleaner, MemReclaimer
- `table/` — Tablet, TabletWriter, TabletReader, KVTable, KKVTable, NormalTable
- `file_system/` — Directory, PackageFile, ArchiveFolder, LoadConfig, WAL
- `config/` — Schema, IndexConfig, FieldConfig, BuildConfig, MergeConfig
- `util/` — cache, counter, memory_control, slice_array, buffer_compressor

## Legacy vs IndexlibV2
indexlib has two generations:
- **Legacy** (`indexlib/index_base/`, `indexlib/config/`) — older partition-based API
- **IndexlibV2** (`index/`, `framework/`, `table/`) — newer tablet-based API with ITablet interface
Both coexist; V2 is the forward path.",
        None, &[
            ("aios/storage/indexlib/index/inverted_index/InvertedIndexReader.h", None, None, "Inverted index reader."),
            ("aios/storage/indexlib/index/kkv/KKVSegmentWriter.h", None, None, "KKV segment writer."),
        ]);

    seed_page(&s, "havenask/storage/inverted-index", "Inverted Index", PageKind::Index,
        "Inverted index: posting list read/write, term dictionary, bitmap index, and segment-level index management.",
        r"## Writing Path
- **`PostingWriter`** (base) → **`PostingWriterImpl`** — encodes doc-ids, TF, positions into posting lists
  - `EndSegment()` — finalizes posting data and writes to disk
  - `GetDumpLength()` — returns serialized size
  - `GetDF()` / `GetTotalTF()` — document frequency and total term frequency
  - `SpeedUpType` enum: `NONE`, `BITMAP` — controls bitmap acceleration
- **`InvertedIndexBuildWorkItem`** — builds inverted index for a segment
- **`NormalIndexWriter::EstimateInitMemUse()`** — estimates memory for index build
- **`DictionaryWriter`** — writes term → offset dictionary

## Reading Path
- **`InvertedIndexReader`** — looks up term in dictionary, creates PostingIterator
  - `IndexSegmentReader` — per-segment index reader
  - `KeyIterator` — iterates terms in the dictionary
  - `IndexUpdateTermIterator` — incremental term updates
  - `SegmentPosting` — per-segment posting reference (used by iterator)
- **`PostingIterator`** (base, in_degree=11, most connected class) → **`BufferedPostingIterator`** — decodes and seeks posting lists
  - `Init(SegmentPostingVector&)` — initializes with segment posting list
  - `SeekDoc(docid_t)` / `SeekDocWithErrorCode()` — seek to target doc
  - `GetMatchValue()` — returns match value (term weight, position)
- **`MultiSegmentPostingIterator`** — iterates across segments
  - `Init()`, `Reset()`, `SeekDoc()`, `SeekDocWithErrorCode()`
- **`BitmapIndexReader`** — bitmap-based inverted index for low-cardinality fields

## Posting Format
Skiplist-based compressed posting lists with section-weight encoding for positional information.

## Bitmap Posting
- **`BitmapPostingMerger::CreatePostingIterator()`** (in_degree=4) — creates bitmap posting iterator
- **`MultiSegmentBitmapPostingWriter`** — bitmap posting across segments
  - `EndSegment()`, `GetDF()` (in_degree=4 each)",
        None, &[
            ("aios/storage/indexlib/index/inverted_index/PostingWriter.h", None, None, "PostingWriter base class."),
            ("aios/storage/indexlib/index/inverted_index/PostingWriterImpl.h", None, None, "PostingWriterImpl: EndSegment(), GetDF(), SpeedUpType."),
            ("aios/storage/indexlib/index/inverted_index/BufferedPostingIterator.h", None, None, "BufferedPostingIterator: Init(), SeekDoc()."),
            ("aios/storage/indexlib/index/inverted_index/SegmentPosting.h", None, None, "SegmentPosting: per-segment posting reference."),
            ("aios/storage/indexlib/index/inverted_index/InvertedIndexBuildWorkItem.h", None, None, "Build work item."),
        ]);

    seed_page(
        &s,
        "havenask/storage/attribute",
        "Attribute Index",
        PageKind::Index,
        "Columnar attribute storage: read, write, patch, format, and merge.",
        r"## Architecture
- **`AttributeReader`** — reads attribute values by doc-id
  - Supports typed access: `AttributeReaderTyped<T>::Read(docid_t, T&)`
  - Multi-value attributes return packed value arrays
- **`AttributeWriter`** — writes attribute values during build
  - `AttributeWriterTyped<T>::SetValue(docid_t, const T&)`
  - Buffer management with memory quota
- **`AttributePatch`** — applies incremental patches to built segments
  - Patches are generated by real-time document updates
  - Applied lazily during read or merged during segment merge
- **`AttributeMerger`** — merges attribute data across segments
  - Called during segment merge to combine attribute columns
- **`AttributeConfig`** — field type, compression, default value configuration
- **`AttributeFormat`** — on-disk format variants:
  - Plain: fixed-size, direct offset
  - Compressed: LZ4/ZSTD compressed blocks
  - Offset: variable-length with offset table
- **`ExpressionAttribute`** — expression-based virtual attributes
  - Computed at read-time from other attributes

## Subdirectories
- `config/` — AttributeConfig, AttributeType definitions
- `expression/` — expression-based virtual attributes
- `format/` — on-disk format implementations
- `merger/` — attribute merge logic
- `patch/` — patch application logic
- `test/` — unit tests",
        None,
        &[
            (
                "aios/storage/indexlib/index/attribute/AttributeReader.h",
                None,
                None,
                "AttributeReader: Read(docid_t, T&).",
            ),
            (
                "aios/storage/indexlib/index/attribute/AttributeWriter.h",
                None,
                None,
                "AttributeWriter: SetValue(docid_t, T&).",
            ),
        ],
    );

    seed_page(
        &s,
        "havenask/storage/kv-kkv",
        "KV / KKV Index",
        PageKind::Index,
        "Primary-key store: KV (single-value) and KKV (multi-value, prefix-compressed).",
        r"## KV (Key-Value)
- **`KVSegmentWriter`** / **`KVSegmentReader`** — per-segment KV write/read
- **`HashTablePrimaryKeyDiskIndexer::Lookup()`** — on-disk hash table primary key lookup
  - Uses `PrimaryKeyHashTable::Find(key, value)` for O(1) lookup
  - `PrimaryKeyHashTable::CalculateMemorySize()` — pre-calculates hash table size
  - `PrimaryKeyHashTable::GetBucketCount()` — returns bucket count
- **`HashTableVarSegmentReader::Get()`** — variable-length segment read
- **`HashTableCompressVarSegmentReader::Get()`** — compressed variant

## KKV (Key-Key-Value)
- **`KKVSegmentWriter`** (in_degree=6) — multi-value KKV build
  - Builds prefix key table + suffix value data
- **`KKVReader::LookupAsync()`** — async KKV primary key lookup
- **`ClosedHashPrefixKeyTable`** — closed-hash prefix key storage
  - `CalculateBuildMemoryUse()` / `CalculateMemoryUse()` — memory estimation
  - `EstimateCapacity()` — capacity planning
  - `Release()` — releases memory
- **`ClosedPKeyTableSpecialValueTraits::ToStoreValue()`** — converts special key values

## Hash Table Variants
- **`DenseHashTable`** — fixed-size, fastest lookup; `Find(key, value)` returns Status
  - `DenseHashTableBase` — abstract base with virtual Find/FindForReadWrite
  - Template params: `_KT` (key type), `_VT` (value type), `HasSpecialKey`, `useCompactBucket`
- **`CuckooHashTable`** — space-efficient, guaranteed O(1) worst-case
- **`SeparateChainHashTable`** — chaining for variable-length values

## Value Compression
- **Succinct** — value compression for KKV suffix values
- **PlainFormat** / **CompressedFormat** — format selection per field",
        None,
        &[
            (
                "aios/storage/indexlib/index/kkv/KKVSegmentWriter.h",
                None,
                None,
                "KKV segment writer.",
            ),
            (
                "aios/storage/indexlib/index/common/hash_table/DenseHashTable.h",
                None,
                None,
                "DenseHashTable: Find(), template params.",
            ),
        ],
    );

    seed_page(
        &s,
        "havenask/storage/segment",
        "Segment Lifecycle",
        PageKind::Article,
        "Segment is the basic unit of index storage: building → dump → built lifecycle.",
        r"## Lifecycle
1. **Building** — segment under construction (in-memory), accepts document writes
   - Documents are buffered in memory
   - `BuildingIndexReader` provides read access to in-progress data
2. **Dump** — segment being persisted to disk (`AsyncSegmentDumper`)
   - `DumpSegmentContainer` / `DumpSegmentQueue` — pending dump management
     - `GetReclaimTimestamp()` — returns timestamp for reclaim
     - `GetEstimateDumpSize()` — estimates dump size
   - `CustomSegmentDumpItem::GetSegmentInfo()` — per-dump segment info
   - `AsyncSegmentDumper` — coordinates async dump
   - `DumpSegmentExecutor` — executes dump tasks
3. **Built** — segment fully built and searchable (read-only)
   - `OnDiskSegmentReader` — reads persisted segment data

## Segment Iteration
- **`PartitionSegmentIterator`** — iterates segments in a partition
  - `CreateIterator()` — creates segment iterator
  - `GetBuildingSegmentId()` — returns building segment ID
  - `GetInMemSegment()` — returns in-memory segment reference

## Document Dedup
- **`BuildingSegmentDocumentDeduper`** — dedup within building segment
- **`BuiltSegmentsDocumentDeduper`** — dedup across built segments
- **`CompressRatioCalculator`** (in_degree=4) — calculates compression ratio for dedup

## Merge
Segments can be merged to reduce count and reclaim deleted docs:
- **Merge strategies**: prefix-key merge, KV optimize merge, normal-table merge
- Merge is triggered by `IndexTask` in the framework layer",
        None,
        &[
            (
                "aios/storage/indexlib/indexlib/partition/AsyncSegmentDumper.h",
                None,
                None,
                "AsyncSegmentDumper.",
            ),
            (
                "aios/storage/indexlib/indexlib/partition/DumpSegmentContainer.h",
                None,
                None,
                "Dump segment container.",
            ),
        ],
    );

    seed_page(&s, "havenask/storage/partition", "Partition", PageKind::Article,
        "Partition is the online serving unit: manages segments, reads, writes, and dump coordination.",
        r"## Architecture
- **`IndexPartition`** — legacy partition abstraction
- **`OnlinePartition`** — partition in search service
  - Reads from built segments + building segment simultaneously
  - Supports real-time document addition and search
- **`PartitionSegmentIterator`** — iterates segments in order

## Key Operations
- **Open** — load partition from disk, recover fence state
  - Loads schema, config, and segment data
  - Recovers from crash-consistent state using Version/VersionCoord
- **Build** — add documents to building segment
  - Documents flow through Document pipeline, then into segment writers
- **Dump** — persist building segment to disk
  - Building segment → AsyncSegmentDumper → OnDiskSegment
- **Reopen** — reload after segment dump/merge completes
  - Atomic switch: old readers stay valid, new readers created

## Operation Queue
`OperationQueue` serializes concurrent operations (build, dump, merge, reopen) on a partition. Ensures consistency:
- Build operations are queued sequentially
- Dump/merge operations coordinate with build
- Reopen waits for in-flight reads to complete

## CompressRatioCalculator
`CompressRatioCalculator` (in_degree=4) calculates document compression ratio for partition sizing and resource planning.",
        None, &[
            ("aios/storage/indexlib/indexlib/partition/OnlinePartition.h", None, None, "Online partition."),
        ]);

    seed_page(&s, "havenask/storage/tablet", "Tablet Framework", PageKind::Index,
        "Tablet is the next-gen table abstraction replacing legacy partition: ITablet, TabletWriter, TabletReader.",
        r"## ITablet Interface
```
class ITablet : private autil::NoCopyable {
    virtual ~ITablet() = default;
    virtual Status Open(const IndexRoot&, shared_ptr<ITabletSchema>, const TabletOptions&) = 0;
    virtual Status Reopen(const ReopenOptions&, const VersionCoord&) = 0;
    virtual void Close() = 0;
    virtual Status Build(shared_ptr<IDocumentBatch>) = 0;
};
```

## Key Classes
- **`ITablet`** — tablet interface (abstract)
- **`Tablet`** — concrete tablet implementation
- **`TabletWriter`** — writes documents into tablet
- **`TabletReader`** — reads from tablet (search, lookup)
- **`TabletFactory`** — creates tablet components
- **`TabletSchema`** (ITabletSchema) — schema definition for tablet
- **`Version`** / **VersionCoord`** — version management for atomic switch

## Framework Sub-modules
- **LifecycleTablet** — lifecycle management (open, close, seal)
- **IndexTask** — background index tasks (merge, compact, add index)
  - `index_task/testlib/` — test utilities for index tasks
- **Cleaner** — cleans obsolete segment files
- **MemReclaimer** — reclaims unused memory
- **Hooks** — lifecycle hooks for custom behavior

## Tablet vs Partition
Tablet replaces the legacy Partition API:
- **Partition**: single-process, no schema evolution, indexlibV1
- **Tablet**: schema evolution, IndexTask framework, indexlibV2
- Both coexist; Tablet is the forward path",
        None, &[
            ("aios/storage/indexlib/framework/ITablet.h", None, None, "ITablet interface: Open(), Reopen(), Close(), Build()."),
            ("aios/storage/indexlib/table/Tablet.h", None, None, "Tablet implementation."),
        ]);

    seed_page(&s, "havenask/storage/document", "Document Processing", PageKind::Article,
        "Document layer: raw document parsing, tokenization, KV/KKV document formatting, and rewriting.",
        r"## Pipeline
1. **RawDocument** → parsed from source (Ha3RawDocument, FbRawDocument)
   - `RawDocument` — key-value pair container for raw document fields
   - `FbKvRawDocumentParser` — FlatBuffer KV format parser
     - `initFieldProperty()` (in_degree=8) — initializes field properties
     - `initKeyHasher()` (in_degree=8) — initializes key hash function
2. **DocumentRewriter** → modifies document fields before indexing
   - `NormalDocumentRewriter` — rewrites for normal (inverted+attribute) index
   - `KVDocumentRewriter` — rewrites for KV index
3. **Tokenize** → tokenizes text fields for inverted index
   - `TokenizeSection` / `TokenizeField` — tokenize result containers
4. **NormalDocument** → structured document with fields, sections
   - `NormalDocument` — contains AttributeDocument, IndexDocument, SummaryDocument
5. **KV/KKV Document** → key-value formatted document
   - `KVDocument` / `KKVDocument` — specialized for KV/KKV index

## Sub-directories
- `raw_document/` — RawDocument, RawDocumentParser
- `kv/` — KV document format
- `kkv/` — KKV document format
- `normal/` — NormalDocument, tokenization, rewriter
- `extractor/` — FieldTokenExtractor, index field extraction
- `document_rewriter/` — document rewriting pipeline",
        None, &[
            ("aios/storage/indexlib/document/raw_document/RawDocument.h", None, None, "RawDocument base."),
            ("aios/storage/indexlib/document/normal/NormalDocument.h", None, None, "NormalDocument."),
        ]);

    seed_page(
        &s,
        "havenask/storage/file-system",
        "File System",
        PageKind::Article,
        "indexlib file system: package file, archive, flush, load config, and relocatable storage.",
        r"## Architecture
- **`Directory`** — logical directory abstraction (created by FileSystem)
- **`PackageFile`** — packages multiple index files into one for efficient IO
  - Reduces file count and improves sequential read performance
  - `package/` — package file writer/reader
- **`ArchiveFolder`** — archive-based storage backend
- **`FileReader` / `FileWriter`** — file IO with cache support
  - `file/` — file implementations
  - `stream/` — streaming file access
- **`LoadConfig`** — configures how index files are loaded
  - `load_config/` — load config parsing and management
  - Options: mmap, block-cache, direct-read
- **`Relocatable`** — supports file relocation across storage tiers
  - `relocatable/` — relocatable file management
- **`WAL`** — write-ahead log for durability during build
  - `wal/` — WAL implementation
- **`Flush`** — flush strategy for in-memory data to disk
  - `flush/` — flush implementations
- **`fslib/`** — file system library abstractions
- **`mock/`** — mock file system for testing

## Package File Format
Package files combine many small index files into a single large file:
- Header: file name → offset map
- Body: concatenated file data
- Benefits: fewer open() calls, better sequential IO, less inode usage",
        None,
        &[(
            "aios/storage/indexlib/file_system/Directory.h",
            None,
            None,
            "Directory abstraction.",
        )],
    );

    seed_page(
        &s,
        "havenask/storage/config",
        "Index Config",
        PageKind::Article,
        "Index configuration: schema, field type, index settings, and build options.",
        r"## Key Configs
- **`Schema`** — defines fields, indexes, attributes, summary
  - Top-level configuration for an index table
  - Contains: IndexSchema, AttributeSchema, SummarySchema
- **`IndexConfig`** — per-index configuration
  - InvertedIndexConfig: analyzer, dictionary type, posting format
  - KVConfig / KKVConfig: key type, value type, hash method
  - AttributeConfig: compression, default value
  - SummaryConfig: group config, compression
- **`FieldConfig`** — field type, analyzer, default value
  - FieldType: INT32, UINT32, INT64, UINT64, FLOAT, DOUBLE, STRING, TEXT, etc.
- **`BuildConfig`** — build parameters
  - Realtime vs offline build mode
  - Max doc count per segment
  - Build memory quota
- **`MergeConfig`** — merge strategy and parameters
  - MergeStrategy: prefix-key, optimize, normal
  - MergePlan: level merge, classify by doc count
- **`LoadConfig`** — index file load strategy
  - LoadMode: MMAP, CACHE, DIRECT_READ
  - Cache size and eviction policy",
        None,
        &[],
    );

    // ════════════════════════════════════════════════════════════════
    // Level 1: SQL / Iquan
    // ════════════════════════════════════════════════════════════════
    seed_page(
        &s,
        "havenask/sql",
        "SQL / Iquan",
        PageKind::Index,
        "SQL-over-search: parse SQL → Iquan optimize → Navi graph execution → HA3 search.",
        r"## Pipeline
1. **`SqlParseKernel`** — parses SQL statement into logical plan
   - `compute()` — main compute method
2. **`Iquan`** — query optimizer (cost-based, rule-based)
   - CatalogDef for metadata lookup
   - Join reorder and plan transformation
3. **`PlanTransformKernel`** — transforms optimized plan into Navi graph
   - `compute()` — transforms and outputs Navi graph spec
4. **`Navi`** — executes the DAG graph
5. **`SqlClientInfoKernel`** — collects client info for tracing
   - `compute()` — adds tracing metadata

## Key Modules
- **Iquan** — optimizer engine with catalog, join reorder, and plan transformation
  - `CatalogDef::isValid()` — validates catalog definition
  - `AliasRef` — expression reference and binding
  - `IQUAN_ENSURE_FUNC` (in_degree=62, most connected macro) — plan validation
- **SQL Ops** — Navi kernels: agg, join, sort, scan, seek, table scan
- **SQL Config** — SQL service configuration
- **SQL Common** — shared types, reference, expression binding
- **SQL Proto** — SQL service protocol buffers
- **SQL Framework** — SQL query framework and session management

## Directory Structure
`aios/sql/`:
- `ops/` — ScanKernel, SeekKernel, JoinKernel, AggKernel, SortKernel, IdentityKernel
- `iquan/` — optimizer, catalog, plan transform
- `framework/` — SqlFramework, session management
- `config/` — SQL service configuration
- `common/` — shared types
- `proto/` — protobuf definitions",
        None,
        &[(
            "aios/sql/ops/SqlOpsKernel.h",
            None,
            None,
            "SQL ops kernel base.",
        )],
    );

    seed_page(&s, "havenask/sql/iquan", "Iquan Optimizer", PageKind::Article,
        "Iquan is the cost-based query optimizer for SQL-over-search: plan transformation, join reorder, catalog management.",
        r"## Architecture
- **`CatalogDef`** — table and function metadata catalog
  - `isValid()` — validates catalog definition (checks table, function, database integrity)
- **`AliasRef`** — expression reference and binding
  - `AliasRef::AliasRef()` — constructor with reference info
  - `reference_logging()` — logs reference resolution for debugging
- **`PlanTransformKernel`** — transforms Iquan plan into Navi graph
  - `PlanTransformKernel()` — constructor
  - `compute()` — main transformation logic
- **`IQUAN_ENSURE_FUNC`** (in_degree=62) — validation macro for plan constraints
  - Most connected macro in the SQL layer, used pervasively for plan validation

## Optimization Rules
- **Rule-based**: predicate pushdown, projection pruning, constant folding
- **Cost-based**: join reorder based on table statistics, index selection
- **Physical**: kernel selection for each logical operator (scan vs seek, hash join vs nested loop)

## Plan Lifecycle
1. Logical plan from SQL parser
2. Rule-based rewrite (predicate pushdown, etc.)
3. Cost-based optimization (join reorder)
4. Physical plan generation (kernel selection)
5. Plan → Navi graph transformation",
        None, &[
            ("aios/sql/iquan/cpp/common/Common.h", None, None, "Iquan common definitions with IQUAN_ENSURE_FUNC."),
        ]);

    seed_page(&s, "havenask/sql/ops", "SQL Ops (Navi Kernels)", PageKind::Article,
        "SQL ops are Navi kernel implementations for each SQL operation: scan, join, agg, sort, seek.",
        r"## Key Ops
- **`ScanKernel`** — table scan with pushed-down predicates
  - `ScanIteratorCreatorR::init()` — initializes scan iterator
  - Supports: full table scan, index scan, bitmap scan
- **`SeekKernel`** — index seek (term lookup)
  - Direct term lookup in inverted index
- **`JoinKernel`** — hash join, nested-loop join
  - `compute()` — performs join on input streams
- **`AggKernel`** — aggregation (SUM, COUNT, AVG, MIN, MAX)
  - `compute()` — aggregates input data
- **`SortKernel`** — top-K sort
  - `compute()` — sorts and truncates to top-K
- **`IdentityKernel`** — pass-through for debug
  - `compute()` — passes data through unchanged
- **`RunSqlGraphKernel`** — graph execution entry point
  - Coordinates execution of the full SQL graph

## Directory Structure
`aios/sql/ops/`:
- `scan/` — table scan kernels
- `join/` — join kernels
- `agg/` — aggregation kernels
- `sort/` — sort kernels
- `identity/` — pass-through kernel
- `runSqlGraph/` — graph execution kernel",
        None, &[]);

    // ════════════════════════════════════════════════════════════════
    // Level 1: Navi
    // ════════════════════════════════════════════════════════════════
    seed_page(&s, "havenask/navi", "Navi (DAG Execution)", PageKind::Index,
        "Navi is the DAG graph execution engine: builds computation graphs and executes them with dataflow scheduling.",
        r"## Core Concepts
- **`Graph`** — a computation DAG
  - `isTimeout()` — checks if graph execution has timed out
- **`GraphBuilder`** — builds graph from kernel specifications
  - `node()` — adds a kernel node to the graph
  - `GraphBorder` — boundary between graph and external world
    - `match()` — matches border conditions
- **`Kernel`** — computation unit (like a Spark operator)
  - Each SQL op is a Kernel subclass
  - `compute()` — main computation method
- **`Port`** — input/output edge between kernels
- **`ArpcGraphClosure`** — RPC completion callback
  - Used to send results back to client when graph completes

## Execution Model
1. User defines a graph (kernel nodes + edge connections)
2. Navi schedules kernels based on data availability (dataflow scheduling)
3. Each kernel processes input data and produces output
4. Final results collected via graph closure

## Modules
- **`Navi Config`** — graph configuration loading (Python-based config loader)
- **`Navi Distribute`** — distributed graph execution across processes
- **`Navi RPC Server`** — serves graph execution via RPC
- **`Navi Tester`** — testing framework for graph execution
- **`Navi Logger`** — execution logging and tracing
- **`Navi Util`** — utility functions

## Directory Structure
`aios/navi/`:
- `config/` — graph configuration
- `config_loader/` — Python-based config loader
- `distribute/` — distributed execution
- `rpc_server/` — RPC server
- `tester/` — test framework
- `log/` — logging
- `util/` — utilities
- `example/` — examples",
        None, &[
            ("aios/navi/Graph.h", None, None, "Graph class."),
            ("aios/navi/GraphBuilder.h", None, None, "Graph builder."),
        ]);

    // ════════════════════════════════════════════════════════════════
    // Level 1: Suez
    // ════════════════════════════════════════════════════════════════
    seed_page(&s, "havenask/suez", "Suez (Service Framework)", PageKind::Index,
        "Suez is the service framework: deployment, heartbeat, table management, cluster management.",
        r"## Modules
- **Suez Search** — search service (QRS + Searcher roles)
- **Suez Deploy** — deployment decision making
  - `DeployDecisionMaker::rewriteFinal()` (in_degree=4) — finalizes deployment decision
- **Suez Heartbeat** — heartbeat-based config distribution
  - `HeartbeatTarget::Jsonize()` — serializes heartbeat target (in_degree=2)
  - `HeartbeatTarget::setTableMetas()` (in_degree=8) — sets table metadata in heartbeat
- **Suez Admin** — cluster administration
- **Suez Service** — service lifecycle management
- **Suez Worker** — worker process management
- **Suez SDK** — service SDK for client integration
- **Suez Table** — table management within a service
  - `SuezTable` — table wrapper with partition info
  - WAL support for direct write
- **Suez Common** — shared utilities

## Key Classes
- **`HeartbeatTarget`** — heartbeat target with table metas (Jsonizable)
  - Carries: table configs, deploy decisions, version info
  - `setTableMetas()` — highest connectivity method in Suez (in_degree=8)
- **`DeployDecisionMaker`** — decides when and how to redeploy
  - `rewriteFinal()` — final deployment decision after all checks
- **`ClusterServiceImpl`** — cluster service RPC
  - `deployDatabase()` — triggers database deployment
- **`ClusterManager`** — cluster lifecycle management
  - `create_or_update_default_clusters()` (in_degree=4) — creates/updates cluster configs
- **`SuezClusterService`** — Python-accessible cluster service
  - `deploy_database()` / `update_cluster()` / `get_cluster()`

## Service Lifecycle
1. Suez starts worker processes via Admin
2. Heartbeat delivers config to workers
3. DeployDecisionMaker evaluates config changes
4. Workers load/reload tables based on heartbeat target
5. Service becomes ready when tables are loaded",
        None, &[
            ("aios/suez/search", None, None, "Suez search service."),
            ("aios/suez/deploy", None, None, "Suez deploy."),
            ("aios/suez/heartbeat", None, None, "Suez heartbeat."),
        ]);

    // ════════════════════════════════════════════════════════════════
    // Level 1: Build Service
    // ════════════════════════════════════════════════════════════════
    seed_page(&s, "havenask/build-service", "Build Service", PageKind::Index,
        "Build Service is the distributed index build system: builder administration, task scheduling, and worker management.",
        r"## Architecture
- **Admin** — build task coordinator
  - `BuilderTaskWrapper::start()` — starts a builder task (in_degree=2)
  - `GraphBuilderWrapper::openApi()` — graph-based build orchestration (in_degree=2)
  - `SingleBuilderTask::init()` / `notifyStopped()` / `clearFullWorkerZkNode()` — single-phase build task lifecycle
  - `getTaskPhaseIdentifier()` — returns current build phase (full, inc, etc.)
- **Builder** — per-worker build logic
  - `AsyncBuilderV2` — async builder using ITablet interface
  - `BuilderController` — controls build progress and version management
    - Holds `shared_ptr<ITablet>` for index access
    - `loadVersion()` — loads version from progress info
    - `hasFatalError()` — checks for fatal build errors
- **Task Base** — common task abstractions
- **Config** — build configuration (TaskTarget, BuildTaskTargetInfo)
- **Proto** — build service protocol buffers (ErrorCollector)

## Build Flow
1. Admin creates builder task via `BuilderTaskWrapper::start()`
2. Scheduler (Hippo) allocates worker slots via `reAllocRoleSlots()`
3. Workers build index segments using `AsyncBuilderV2` + `ITablet::Build()`
4. `BuilderController` manages version commits
5. Admin coordinates full/incr build phases via `SingleBuilderTask`
6. ZK nodes track worker assignments and progress

## Location
Build Service lives at: `aios/apps/facility/build_service/`",
        None, &[
            ("aios/apps/facility/build_service/build_service/admin/BuilderTaskWrapper.h", None, None, "BuilderTaskWrapper: start()."),
            ("aios/apps/facility/build_service/build_service/builder/AsyncBuilderV2.h", None, None, "AsyncBuilderV2 with ITablet."),
            ("aios/apps/facility/build_service/build_service/build_task/BuilderController.h", None, None, "BuilderController: loadVersion(), hasFatalError()."),
        ]);

    // ════════════════════════════════════════════════════════════════
    // Level 1: Hippo
    // ════════════════════════════════════════════════════════════════
    seed_page(&s, "havenask/hippo", "Hippo (Scheduler)", PageKind::Article,
        "Hippo is the resource scheduler: allocates and manages worker slots for build and search services.",
        r"## Architecture
- **`SimpleMasterScheduler`** — scheduler interface
  - `reAllocRoleSlots()` — reallocates slots based on role requirements
  - `setMasterDriver()` — sets the master driver for slot management
- **`SimpleMasterSchedulerOrigin`** — original scheduler implementation
  - `init()` — initializes with config
  - `releaseSlots()` — releases allocated slots
  - `reAllocRoleSlots()` — reallocates slots
- **`SimpleMasterSchedulerAdapterImpl`** — adapter for scheduler
  - `releaseSlots()` — adapter release
- **`AgentSimpleMasterScheduler`** — agent-based scheduler (used by Build Service)
  - `reAllocRoleSlots()` (in_degree=4) — agent-based reallocation
- **`SimpleMasterSchedulerLocal`** — local mode scheduler for testing
  - `start()` — starts local scheduler

## Key Operations
- **reAllocRoleSlots** — core operation: examines role requirements, available slots, and allocates
- **releaseSlots** — returns slots to the pool when no longer needed
- **init** — initializes scheduler with master driver and configuration

## Hippo Protocol
Hippo uses protobuf for slot allocation and worker management RPCs. The scheduler communicates with a Hippo Master to allocate/deallocate physical resources (CPU, memory, disk).

## Integration with Build Service
Build Service uses `AgentSimpleMasterScheduler` to request build worker slots from Hippo. When a build task needs N workers, the admin calls `reAllocRoleSlots()` which negotiates with Hippo Master.",
        None, &[
            ("aios/hippo/include/hippo/SimpleMasterScheduler.h", None, None, "Scheduler interface."),
        ]);

    // ════════════════════════════════════════════════════════════════
    // Level 1: Catalog
    // ════════════════════════════════════════════════════════════════
    seed_page(&s, "havenask/catalog", "Catalog (Metadata Service)", PageKind::Article,
        "Catalog is the table and config metadata service: manages table structures, databases, and partitions.",
        r"## Entity Model
```
Database → TableGroup → Table → TableStructure → Partition
```
Each entity has:
- `copyDetail()` — copies entity details for update
- `update()` — applies updates
- `toProto()` — converts to protobuf

## Key Entities
- **`Database`** — top-level container for tables
  - `copyDetail()` — deep copy for update
- **`TableGroup`** — groups related tables
  - `copyDetail()` / `update()` — modify table group
- **`Table`** — table entity (inherits EntityBase<Table>)
  - `copyDetail()` — copies table details
- **`TableStructure`** — table schema definition
  - `copyDetail()` / `toProto()` — access and serialize
- **`Partition`** — table partition info
  - `copyDetail()` — copies partition details

## CatalogDef for Iquan
`CatalogDef::isValid()` — validates the catalog definition used by Iquan optimizer for query planning.

## Modules
- **Catalog Entity** — entity model classes (Table, Database, TableGroup, Partition)
- **Catalog Service** — RPC service for catalog operations
- **Catalog Store** — persistent storage backend
- **Catalog Proto** — protocol buffers
- **Catalog Util** — utility functions
- **Catalog Tools** — admin CLI tools
  - `TableStructureConfigBuilder::set_table_meta()` (in_degree=4) — builds table meta for catalog registration",
        None, &[
            ("aios/catalog/entity/Table.h", None, None, "Table entity: EntityBase<Table>."),
            ("aios/catalog/entity/Database.h", None, None, "Database entity."),
        ]);

    // ════════════════════════════════════════════════════════════════
    // Level 1: MatchDoc
    // ════════════════════════════════════════════════════════════════
    seed_page(
        &s,
        "havenask/matchdoc",
        "MatchDoc",
        PageKind::Article,
        "MatchDoc is the document reference and allocation framework used throughout HA3 search.",
        r"## Core Concepts
- **`MatchDoc`** — lightweight document reference (docid + allocator pointer)
  - Cheap to copy and pass around
  - Actual data stored in allocator's memory pool
- **`MatchDocAllocator`** — allocates and manages matchdoc storage
  - `getReferenceCount()` (in_degree=8) — count of references for a field
  - `clearFieldGroups()` (in_degree=8) — clears allocated field groups
  - `truncateSubDoc()` (in_degree=8) — truncates sub-documents
  - `renameField()` — renames a field reference
  - `toDebugString()` — debug output
  - `getSubReferenceCount()` (in_degree=4) — sub-doc reference count
  - `setSortRefFlag()` (in_degree=4) — marks reference for sorting
  - Internal: `FieldGroup` and `SubDocAccessor` for field management
- **`Reference<T>`** — typed reference to a field in matchdoc
  - `setIsMountReference()` (in_degree=4) — marks as mountable reference
- **`ReferenceBase`** — untyped reference base

## FlatBuffer Support
MatchDoc supports FlatBuffer serialization (in `matchdoc/flatbuffer/`) for efficient IPC between QRS and Searcher nodes. This avoids protobuf overhead for high-throughput data transfer.

## Toolkit
`matchdoc/toolkit/` provides debug and inspection utilities for MatchDoc allocation.",
        None,
        &[
            (
                "aios/matchdoc/MatchDoc.h",
                None,
                None,
                "MatchDoc definition.",
            ),
            (
                "aios/matchdoc/MatchDocAllocator.h",
                None,
                None,
                "MatchDocAllocator with FieldGroup and SubDocAccessor.",
            ),
        ],
    );

    // ════════════════════════════════════════════════════════════════
    // Level 1: Expression
    // ════════════════════════════════════════════════════════════════
    seed_page(&s, "havenask/expression", "Expression Framework", PageKind::Article,
        "Expression framework: syntax parsing, function registry, and attribute expression evaluation.",
        r"## Class Hierarchy
- **`SyntaxExpr`** (abstract base) — syntax expression node
  - `accept(SyntaxExprVisitor*)` — visitor pattern
  - `serialize()` / `deserialize()` — data buffer serialization
  - Subclasses: `AtomicSyntaxExpr`, `BinarySyntaxExpr`, `FuncSyntaxExpr`, `MultiParamSyntaxExpr`
- **`SyntaxExprVisitor`** — visitor interface for syntax tree
- **`SyntaxExpressionFactory`** — creates and manages syntax expressions
  - `init()` — initializes factory
  - `destroy()` — cleanup
  - `needInit()` (in_degree=6) — checks if factory needs initialization
  - `registeFunction()` (in_degree=6) — registers a function with the factory
- **`SyntaxExpr2AttrExpr`** — converts syntax expression to attribute expression
  - Bridges the syntax layer to the runtime evaluation layer

## Expression Types
- **Atomic**: `ConstSyntaxExpr`, `AttrSyntaxExpr`, `SubDocSyntaxExpr`
- **Binary**: `AddSyntaxExpr`, `SubSyntaxExpr`, `MulSyntaxExpr`, `DivSyntaxExpr`
- **Logical**: `AndSyntaxExpr`, `OrSyntaxExpr`, `EqualSyntaxExpr`, `NotEqualSyntaxExpr`
- **Function**: `FuncSyntaxExpr` — function call with parameters

## Function Registry
`SyntaxExpressionFactory::registeFunction()` registers custom functions:
- Built-in: abs, sqrt, pow, if, case, string functions
- Custom: user-defined via Plugins (UDF plugins)

## Directory Structure
`aios/expression/expression/`:
- `syntax/` — SyntaxExpr hierarchy
- `function/` — FunctionInterface, built-in functions
- `framework/` — ExpressionFramework
- `plugin/` — expression plugin interface
- `common/` — shared utilities
- `util/` — utility functions",
        None, &[
            ("aios/expression/expression/syntax/SyntaxExpr.h", None, None, "SyntaxExpr: accept(), serialize()."),
            ("aios/expression/expression/function/FunctionInterface.h", None, None, "Function interface."),
        ]);

    // ════════════════════════════════════════════════════════════════
    // Level 1: Protocol
    // ════════════════════════════════════════════════════════════════
    seed_page(
        &s,
        "havenask/protocol",
        "Protocol",
        PageKind::Article,
        "RPC protocol definitions for inter-service communication.",
        r"## Key Protos
- **Ha3SearchService** — search RPC (SearchRequest → SearchResponse)
- **BuildService** — build service RPC (TaskTarget, BuildTaskTargetInfo)
- **CatalogService** — catalog RPC (Table, Database, Partition CRUD)
- **HippoService** — scheduler RPC (SlotAllocation, WorkerStatus)
- **SuezService** — suez service RPC (HeartbeatTarget, DeployDecision)

## ARPC Transport
All services use ARPC (Alibaba RPC) for communication, built on top of protobuf. ARPC provides:
- Synchronous and async RPC
- Connection pooling
- Timeout and retry
- Server-side streaming

## Error Handling
Build Service uses `ErrorCollector` pattern:
- `proto::ErrorCollector` — base class for error collection
- `BuilderController` inherits ErrorCollector for build error tracking",
        None,
        &[],
    );

    // ════════════════════════════════════════════════════════════════
    // Level 1: Plugins
    // ════════════════════════════════════════════════════════════════
    seed_page(&s, "havenask/plugins", "Plugins", PageKind::Article,
        "Havenask plugin system: analyzer plugins, UDF plugins, and scorer plugins for extensibility.",
        r"## Plugin Types
- **Analyzer Plugins** — custom tokenizers and analyzers for text processing
  - Located: `aios/plugins/havenask_plugins/analyzer_plugins/`
  - Implement `Analyzer` interface
  - Examples: CJK analyzer, space analyzer, single-word analyzer
- **UDF Plugins** — user-defined functions for SQL and expression evaluation
  - Located: `aios/plugins/havenask_plugins/udf_plugins/`
  - Implement `FunctionInterface`
  - Registered via `SyntaxExpressionFactory::registeFunction()`
- **Scorer Plugins** — custom scoring models for ranking
  - Implement `Scorer` interface
  - Used by Turing/CavaScorerAdapter for custom scoring

## Registration
Plugins are loaded dynamically via `PluginManager`:
1. Plugin .so files placed in plugin directory
2. Service config specifies plugin paths
3. PluginManager loads and initializes plugins at startup
4. Each plugin implements a well-known interface and registers itself

## Plugin Directory
`aios/plugins/havenask_plugins/`:
- `analyzer_plugins/` — text analyzer implementations
- `udf_plugins/` — user-defined function implementations",
        None, &[
            ("aios/plugins/havenask_plugins/analyzer_plugins", None, None, "Analyzer plugins."),
            ("aios/plugins/havenask_plugins/udf_plugins", None, None, "UDF plugins."),
        ]);

    // ════════════════════════════════════════════════════════════════
    // Level 1: HA3 Search Layer
    // ════════════════════════════════════════════════════════════════
    seed_page(&s, "havenask/ha3", "HA3 Search Layer", PageKind::Index,
        "HA3 is the query-facing search layer: query parsing, query AST, executor creation, document matching, filtering, and ranking.",
        r"## Key Abstractions
- **QueryParser** → parses search string into expression tree
- **Common Query Model** → AndQuery, OrQuery, TermQuery, PhraseQuery, RankQuery
- **QueryExecutorCreator** → maps query objects + index readers into executors
- **SingleLayerSearcher** → drives matching with layer metadata, filters, match data
- **Rank Comparators** → ComboComparator, ReferenceComparator, MatchDocPriorityQueue
- **Turing** → ML inference (Cava/JIT scoring, TensorFlow models)

## Directory Structure
`aios/ha3/ha3/` contains the core HA3 source code:
- `queryparser/` — QueryParser, ParserContext, QueryExpr hierarchy
- `common/query/` — Query, TermQuery, AndQuery, OrQuery, PhraseQuery, RankQuery, MultiTermQuery, QueryVisitor
- `search/` — QueryExecutor, QueryExecutorCreator, SingleLayerSearcher, Filter, AuxiliaryChain
- `rank/` — Comparator, ComboComparator, ReferenceComparator, MatchDocPriorityQueue
- `turing/` — CavaScorerAdapter, ModelBiz, TuringCommon
- `proto/` — Ha3SearchService proto, SearchRequest/SearchResponse
- `util/` — HA3 utility classes

## QRS vs Searcher
HA3 runs in two roles:
- **QRS** (Query Result Server) — receives requests, parses query, distributes to searchers, aggregates results
- **Searcher** — performs actual index lookup, matching, filtering, and ranking

Communication between QRS and Searcher uses ARPC with protobuf-encoded messages.",
        None, &[
            ("aios/ha3/ha3/isearch.h", None, None, "Top-level HA3 public include."),
            ("aios/ha3/ha3/search/query_executor/QueryExecutor.h", None, None, "Core executor abstraction."),
        ]);

    // ── HA3 sub-pages ──────────────────────────────────────────────
    seed_page(
        &s,
        "havenask/ha3/query-pipeline",
        "Query Pipeline",
        PageKind::Article,
        "How a HA3 query moves from parsed expression to executable search behavior.",
        r"## Flow
1. Query text → `QueryParser::evaluateQuery()` → expression tree (QueryExpr subclasses)
2. Expression tree → common query objects via `QueryExpr::evaluate()`: AndQuery, OrQuery, TermQuery, PhraseQuery, RankQuery
3. `QueryExecutorCreator` takes query objects + `IndexPartitionReaderWrapper` → creates `QueryExecutor` tree
4. `SingleLayerSearcher::seek()` drives matching: iterates doc-id ranges via executor, applies `Filter` chain
5. `MatchDataManager::fillMatchData()` collects match metadata
6. `Comparator::compare()` orders matched docs via `ComboComparator` and `MatchDocPriorityQueue`

## Key Method Signatures
- `QueryParser::evaluateQuery(const std::string&, ParserContext*)` — entry point for query parsing
- `QueryExecutorCreator::create(Query*, IndexPartitionReaderWrapper*)` — creates executor from query
- `SingleLayerSearcher::seek(docid_t)` — seeks to next matching doc
- `SingleLayerSearcher::seekAndJoin(docid_t)` — seeks with hash join support
- `SingleLayerSearcher::constructSubMatchDocs()` — builds sub-document match results (in_degree=10, most complex method)

## Error Handling
QueryExecutor methods return `ErrorCode` rather than throwing, allowing partial-failure semantics in distributed search.",
        None,
        &[
            (
                "aios/ha3/ha3/queryparser/QueryParser.h",
                None,
                None,
                "Parser entry point.",
            ),
            (
                "aios/ha3/ha3/common/query/Query.h",
                None,
                None,
                "Common query base model.",
            ),
            (
                "aios/ha3/ha3/search/query_executor/QueryExecutorCreator.h",
                None,
                None,
                "Executor factory.",
            ),
            (
                "aios/ha3/ha3/search/single_layer_searcher/SingleLayerSearcher.h",
                None,
                None,
                "Searcher driver.",
            ),
            (
                "aios/ha3/ha3/rank/comparator/ComboComparator.h",
                None,
                None,
                "Ranking comparator composition.",
            ),
        ],
    );

    seed_page(&s, "havenask/ha3/queryparser", "Query Parser", PageKind::Index,
        "The parser layer turns textual query syntax into expression nodes before the common query model is built.",
        r"## Core Classes
- **`QueryParser`** — main parser class with `evaluateQuery()` (in_degree=8, most connected method in the module)
  - Takes a query string and `ParserContext`, produces a `QueryExpr` tree
  - `isTerminalTermExpr()` determines if an expression is a leaf term
  - Uses `TextVec` typedef for intermediate token lists
- **`ParserContext`** — holds parser state (index name, term list, analyzer results)
- **`QueryExpr`** — base expression class with visitor pattern
  - `AtomicQueryExpr` — leaf expression (term, number)
  - `AndQueryExpr` / `OrQueryExpr` — binary logical expressions
  - `PhraseQueryExpr` — phrase expression
  - `RankQueryExpr` — rank expression
  - `NumberQueryExpr` — numeric expression
  - `MultiTermQueryExpr` — multi-term expression

## Expression Evaluator
- **`DefaultQueryExprEvaluator`** — evaluates QueryExpr tree into Common Query objects
- The evaluator walks the expression tree and creates corresponding Query subclasses (TermQuery, AndQuery, etc.)

## Parsing Flow
1. Tokenize query string into terms
2. Build expression tree from tokens
3. Evaluate expression tree into common query model
4. Pass common query to QueryExecutorCreator",
        None, &[
            ("aios/ha3/ha3/queryparser/QueryParser.h", None, None, "Parser entry class. Key method: evaluateQuery()."),
            ("aios/ha3/ha3/queryparser/ParserContext.h", None, None, "Parser state and context."),
            ("aios/ha3/ha3/queryparser/QueryExpr.h", None, None, "Expression base class with visitor pattern."),
            ("aios/ha3/ha3/queryparser/DefaultQueryExprEvaluator.h", None, None, "Evaluates QueryExpr into Common Query objects."),
        ]);

    seed_page(
        &s,
        "havenask/ha3/common-query",
        "Common Query Model",
        PageKind::Index,
        "The query model stores parsed search intent in reusable logical and term query objects.",
        r"## Class Hierarchy
- **`Query`** (abstract base) — `accept(QueryVisitor*)`, `accept(ModifyQueryVisitor*)`, `clone()`, `getQueryName()`, `getType()`, `serialize()/deserialize()`
  - **`TermQuery`** — single term search with `Term` object
  - **`AndQuery`** — conjunction of sub-queries
  - **`OrQuery`** — disjunction of sub-queries
  - **`AndNotQuery`** — difference (left AND NOT right)
  - **`PhraseQuery`** — phrase search with term positions
  - **`RankQuery`** — rank query (match left, boost by right)
  - **`MultiTermQuery`** — multi-term query with combine op (AND/ORWEAK_AND)
  - **`TableQuery`** — table lookup query
  - **`NumberQuery`** — numeric term query

## Visitor Pattern
- **`QueryVisitor`** — read-only visitor: `visitTermQuery()`, `visitAndQuery()`, `visitOrQuery()`, `visitPhraseQuery()`, etc.
- **`ModifyQueryVisitor`** — modifying visitor for query rewriting

## Serialization
Query objects support `serialize(DataBuffer&)` / `deserialize(DataBuffer&)` for RPC transmission between QRS and Searcher nodes.

## Relationship to Executors
Each Query subclass maps to one or more QueryExecutor subclasses via QueryExecutorCreator. For example, TermQuery → TermQueryExecutor, AndQuery → AndQueryExecutor.",
        None,
        &[
            (
                "aios/ha3/ha3/common/query/Query.h",
                None,
                None,
                "Abstract Query base with visitor pattern, clone, serialize.",
            ),
            (
                "aios/ha3/ha3/common/query/TermQuery.h",
                None,
                None,
                "TermQuery with Term object.",
            ),
            (
                "aios/ha3/ha3/common/query/MultiTermQuery.h",
                None,
                None,
                "MultiTermQuery with combine operation.",
            ),
            (
                "aios/ha3/ha3/common/query/QueryVisitor.h",
                None,
                None,
                "Read-only QueryVisitor interface.",
            ),
        ],
    );

    seed_page(&s, "havenask/ha3/search", "Search Runtime", PageKind::Index,
        "Search runtime contains query executors, filters, auxiliary chain visitors, and the single-layer search driver.",
        r"## Subsystems
- **Query Executors** — implement matching semantics (term, bitmap, phrase, spatial, multi-term, AND/OR/ANDNOT)
- **Filters** — gate matched documents by attribute conditions
- **SingleLayerSearcher** — coordinates layer traversal and match collection
- **Auxiliary Chain** — collects term statistics (DF, TF) during search

## Directory Structure
`aios/ha3/ha3/search/`:
- `query_executor/` — QueryExecutor, QueryExecutorCreator, all executor subclasses, LayerMetas
- `single_layer_searcher/` — SingleLayerSearcher
- `filter/` — Filter, FilterWrapper
- `auxiliary_chain/` — TermDFVisitor, AuxiliaryChain",
        None, &[
            ("aios/ha3/ha3/search/query_executor/QueryExecutor.h", None, None, "Executor base class."),
            ("aios/ha3/ha3/search/filter/Filter.h", None, None, "Filter base class."),
        ]);

    seed_page(&s, "havenask/ha3/search/query-executors", "Query Executors", PageKind::Article,
        "Executor classes turn query semantics into doc-id seeking, matching, and match-data production.",
        r"## QueryExecutor Base Class
```
class QueryExecutor {
    virtual const std::string getName() const = 0;
    virtual void accept(ExecutorVisitor*) const;
    virtual docid_t seek(docid_t docId) = 0;
    virtual ErrorCode seekSubDoc(docid_t docId, ...);
    virtual bool isMainDocHit(docid_t docId) const = 0;
    virtual df_t getDF(GetDFType type) const = 0;
    virtual void reset();
    virtual void moveToEnd();
    virtual void setEmpty();
    virtual DocValueFilter* stealFilter();
};
```

## Executor Families

### Logical Executors
- **`AndQueryExecutor`** — intersects posting lists from sub-executors
- **`OrQueryExecutor`** — union of sub-executor results
- **`AndNotQueryExecutor`** — left minus right
- **`WeakAndQueryExecutor`** — weak AND (threshold-based)
- **`MultiQueryExecutor`** — base for multi-child executors

### Term Executors
- **`TermQueryExecutor`** — single term posting list seek; `getMatchValue()` returns term match data
- **`BitmapTermQueryExecutor`** — bitmap intersection for AND queries on low-cardinality fields
- **`FieldMapTermQueryExecutor`** — field-aware term executor; `doSeek()` is the core seek method
- **`SubFieldMapTermQueryExecutor`** — sub-document field map; `seekSubDoc()` for nested docs
- **`BufferedTermQueryExecutor`** — buffered posting seek
- **`PhraseQueryExecutor`** — phrase matching with position verification
- **`SpatialTermQueryExecutor`** — spatial search support
- **`PrimaryKeyQueryExecutor`** — primary key lookup
- **`SubDocQueryExecutor`** — sub-document matching

## QueryExecutorCreator
```
class QueryExecutorCreator {
    // Creates executor tree from Query tree + IndexPartitionReaderWrapper
    // Maps: TermQuery → TermQueryExecutor
    //       AndQuery  → AndQueryExecutor
    //       OrQuery   → OrQueryExecutor, etc.
};
```",
        Some("Next pass: trace QueryExecutorCreator into each executor family and record which index reader APIs each executor needs."),
        &[
            ("aios/ha3/ha3/search/query_executor/QueryExecutor.h", None, None, "Executor abstraction with seek(), getDF(), isMainDocHit()."),
            ("aios/ha3/ha3/search/query_executor/QueryExecutorCreator.h", None, None, "Factory: Query + IndexPartitionReaderWrapper → QueryExecutor tree."),
            ("aios/ha3/ha3/search/query_executor/TermQueryExecutor.h", None, None, "Term executor with getMatchValue()."),
            ("aios/ha3/ha3/search/query_executor/BitmapTermQueryExecutor.h", None, None, "Bitmap term executor for low-cardinality fields."),
            ("aios/ha3/ha3/search/query_executor/MultiQueryExecutor.h", None, None, "Composite executor base."),
        ]);

    seed_page(&s, "havenask/ha3/search/single-layer-searcher", "Single Layer Searcher", PageKind::Article,
        "SingleLayerSearcher is the runtime coordinator: layer metadata, query executor, filters, and match data.",
        r"## Class Overview
```
class SingleLayerSearcher {
    // Core methods:
    docid_t seek(docid_t docId);                    // seek to next match
    docid_t seekAndJoin(docid_t docId);              // seek with hash join
    void constructSubMatchDocs();                    // build sub-doc matches (in_degree=10)
    uint64_t getSeekDocCount();                      // stats: docs examined
    using DocMapAttrIterator = ...;                  // attribute iteration typedef
};
```

## Key Interactions
1. Accepts a `QueryExecutor` and `LayerMetas` (doc-id range per layer)
2. Iterates over layer doc-id ranges calling `QueryExecutor::seek()`
3. Applies `Filter` chain per matched document (via `FilterWrapper`)
4. Calls `MatchDataManager::fillMatchData()` to collect match metadata
5. Stores results as `MatchDoc` references in `MatchDocAllocator`

## Layer Metadata
`LayerMetas` defines doc-id ranges for each search layer. A partition may have multiple layers (e.g., real-time + offline). The searcher processes one layer at a time.

## Sub-Document Support
`constructSubMatchDocs()` handles sub-document matching. This is the most complex method (in_degree=10), dealing with nested document structures where a parent doc contains multiple sub-docs that need independent matching.",
        Some("This page should become the main runtime-flow page after a deeper code pass."),
        &[
            ("aios/ha3/ha3/search/single_layer_searcher/SingleLayerSearcher.h", None, None, "Searcher class with seek(), seekAndJoin(), constructSubMatchDocs()."),
            ("aios/ha3/ha3/search/query_executor/LayerMetas.h", None, None, "Layer doc-id range metadata."),
            ("aios/ha3/ha3/search/filter/FilterWrapper.h", None, None, "Filter integration wrapper."),
        ]);

    seed_page(
        &s,
        "havenask/ha3/search/filter",
        "Filters",
        PageKind::Article,
        "Filters gate matched documents by attribute conditions before ranking.",
        r"## Filter Base Class
```
class Filter {
    // Evaluates whether a matched document passes attribute conditions
    // Called by SingleLayerSearcher after each QueryExecutor::seek() hit
};
```

## Filter Hierarchy
- **`Filter`** — base class with evaluation interface
- **`FilterWrapper`** — integrates filter with `AttributeExpressionCreatorBase` and `SyntaxExpr`
  - Binds filter to actual attribute readers
  - Created during search initialization from filter configurations
- **`AttributeFilter`** — range / set / comparison on attribute fields

## Integration with Searcher
1. SingleLayerSearcher receives a `Filter` instance
2. After each `seek()` returns a matching doc-id, the filter is evaluated
3. If the filter rejects the document, match-data construction is skipped (short-circuit optimization)
4. Only accepted documents proceed to `fillMatchData()` and ranking

## Performance
Filter evaluation is critical-path code. Short-circuiting avoids expensive match-data construction for documents that would be filtered out anyway.",
        None,
        &[
            (
                "aios/ha3/ha3/search/filter/Filter.h",
                None,
                None,
                "Filter base class.",
            ),
            (
                "aios/ha3/ha3/search/filter/FilterWrapper.h",
                None,
                None,
                "Filter wrapper binding to attribute readers.",
            ),
        ],
    );

    seed_page(
        &s,
        "havenask/ha3/search/auxiliary-chain",
        "Auxiliary Chain",
        PageKind::Article,
        "Auxiliary chain visitors collect term-level statistics (DF, TF) during search execution.",
        r"## TermDFVisitor
```
class TermDFVisitor : public common::QueryVisitor {
    // Visits each query node to collect document frequency per term
    // Visit methods: visitTermQuery(), visitAndQuery(), visitOrQuery(),
    //   visitPhraseQuery(), visitRankQuery(), visitMultiTermQuery(), etc.
};
```

## How It Works
1. After matching, the auxiliary chain is executed on the query tree
2. `TermDFVisitor` walks the query tree and records DF per term
3. DF statistics feed into BM25 scoring and relevance feedback
4. Other visitors (TF, position) can be chained

## Visitor Chain
- `AuxiliaryChain` — container for a chain of visitors
- Each visitor implements `QueryVisitor` interface
- Visitors are executed sequentially after each match batch",
        None,
        &[(
            "aios/ha3/ha3/search/auxiliary_chain/TermDFVisitor.h",
            None,
            None,
            "Term DF visitor: walks Query tree to collect document frequency.",
        )],
    );

    seed_page(&s, "havenask/ha3/rank", "Ranking Comparators", PageKind::Index,
        "The rank comparator layer orders matchdocs using comparator composition and priority queue support.",
        r"## Comparator Base
```
class Comparator {
    virtual ~Comparator();
    virtual bool compare(matchdoc::MatchDoc a, matchdoc::MatchDoc b) const = 0;
    virtual std::string getType() const;
};
class MatchDocComp {
    // Wraps Comparator for STL sort compatibility
};
```

## ComboComparator Hierarchy
```
Comparator
└── ComboComparator : public Comparator
    ├── OneRefComparatorTyped     — sort by 1 reference field
    ├── TwoRefComparatorTyped     — sort by 2 reference fields
    ├── RefAndExprComparatorTyped — sort by reference + expression
    ├── ThreeRefComparatorTyped   — sort by 3 reference fields
    └── TwoRefAndExprComparatorTyped — 2 references + expression
```

## Key Classes
- **`ComboComparator`** — composes multiple `Reference` and `AttributeExpressionTyped` comparisons; `compare()` evaluates fields in priority order
- **`ReferenceComparator`** — compares a single `matchdoc::Reference<T>` field value
- **`MatchDocPriorityQueue`** — maintains top-K candidates using a heap; avoids sorting all matches
- **`Reference`** (matchdoc) — typed field accessor on MatchDoc

## Ranking Flow
1. SingleLayerSearcher collects matched docs into MatchDoc vector
2. MatchDocPriorityQueue maintains top-K using Comparator for ordering
3. ComboComparator evaluates multi-field sort key (e.g., score DESC, timestamp DESC)
4. Top-K results passed to QRS for aggregation",
        None, &[
            ("aios/ha3/ha3/rank/comparator/Comparator.h", None, None, "Comparator base: virtual compare(a, b)."),
            ("aios/ha3/ha3/rank/comparator/ComboComparator.h", None, None, "Comparator composition with 1/2/3 ref variants."),
            ("aios/ha3/ha3/rank/comparator/MatchDocPriorityQueue.h", None, None, "Top-K heap using Comparator."),
            ("aios/ha3/ha3/rank/comparator/ReferenceComparator.h", None, None, "Single Reference<T> field comparator."),
        ]);

    seed_page(&s, "havenask/ha3/turing", "Turing (ML Inference)", PageKind::Article,
        "HA3 Turing integrates ML inference into the search pipeline via Cava/JIT and TensorFlow model scoring.",
        r"## Architecture
- **TuringCommon** — shared ML inference utilities
- **CavaScorerAdapter** — JIT-compiled scoring via Cava VM
  - Cava is a sandboxed scripting language for custom scoring logic
  - CavaScorerAdapter compiles Cava scripts to native code at runtime
  - Scores are computed per-MatchDoc during the rank phase
- **ModelBiz** — TensorFlow model lifecycle management
  - Loads TF models from distributed storage
  - Manages model versioning and hot-reload
- **SuezTuring** — turing expression integration with Suez service framework
  - `SyntaxExpressionFactory::registeFunction()` registers custom ML functions
  - `SyntaxExpr2AttrExpr` converts ML expressions to attribute expressions

## Scoring Pipeline
1. Standard HA3 matching produces candidate MatchDocs
2. Turing scorer is invoked as a rank-phase plugin
3. CavaScorerAdapter or TF model computes scores per doc
4. Score stored as a `Reference<float>` on MatchDoc
5. ComboComparator uses the score reference for final ordering

## Cava VM
Cava is a Java-like scripting language compiled to native code. Key features:
- Sandboxed execution (no memory safety issues)
- JIT compilation for near-native performance
- Hot-reload without service restart",
        None, &[
            ("aios/ha3/ha3/turing/common/CavaScorerAdapter.h", None, None, "Cava JIT scorer adapter."),
        ]);

    seed_page(
        &s,
        "havenask/ha3/proto",
        "HA3 Proto Definitions",
        PageKind::Article,
        "Protocol buffer definitions for HA3 search service RPC interfaces and data structures.",
        r"## Key Protos
- **Ha3SearchService** — search RPC service definition
  - `search(SearchRequest) → SearchResponse` — main search method
  - `visit(VisitRequest) → VisitResponse` — doc visit for summary
- **SearchRequest** — query text, config name, timeout, partition ranges
- **SearchResponse** — match docs, total hits, seek time, error info

## Wire Format
These protos define the ARPC wire format between QRS and Searcher nodes:
1. QRS serializes SearchRequest → sends to Searcher via ARPC
2. Searcher deserializes, processes, serializes SearchResponse
3. QRS aggregates responses from multiple Searcher partitions

## Configuration Protos
- **SearcherConfig** — searcher-specific configuration
- **QRSConfig** — QRS-specific configuration
- **RankProfile** — rank profile definition",
        None,
        &[],
    );

    println!("{}", out_dir.join("pages/havenask/index.html").display());
    Ok(())
}

fn seed_page(
    store: &WikiStore,
    id: &str,
    title: &str,
    kind: PageKind,
    summary: &str,
    content: &str,
    private_note: Option<&str>,
    evidence: &[(&str, Option<u32>, Option<u32>, &str)],
) -> anyhow::Result<()> {
    store.upsert_page(WikiPage::new(id, title, kind).with_summary(summary))?;
    store.patch_page(id, ContentFormat::Markdown, content, PatchMode::Replace)?;
    if let Some(private_note) = private_note {
        store.patch_private_note(id, private_note, PatchMode::Replace)?;
    }
    for (file, start, end, note) in evidence {
        let mut item = Evidence::new(*file).with_note(*note);
        if let Some(start) = start {
            item.start_line = Some(*start);
            item.end_line = *end;
        }
        store.append_evidence(id, item)?;
    }
    Ok(())
}
