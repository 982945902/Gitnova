#!/usr/bin/env python3
"""MCP stdio client to call gitnova wiki tools for havenask."""
import subprocess
import json
import sys
import os

REPO = "/Users/lishuo121/workspace/havenask"
CMD = ["/Users/lishuo121/Gitnova/target/release/gitnova", "serve"]


def send_recv(proc, msg):
    body = json.dumps(msg)
    header = f"Content-Length: {len(body)}\r\n\r\n"
    proc.stdin.write(header.encode())
    proc.stdin.write(body.encode())
    proc.stdin.flush()
    # read response
    line = proc.stdout.readline()
    if not line:
        return None
    # parse Content-Length
    while line.strip():
        if line.startswith("Content-Length:"):
            length = int(line.split(":")[1].strip())
        line = proc.stdout.readline()
    data = proc.stdout.read(length)
    return json.loads(data)


def call_tool(proc, name, args):
    req = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": name,
            "arguments": args
        }
    }
    return send_recv(proc, req)


def initialize(proc):
    req = {
        "jsonrpc": "2.0",
        "id": 0,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "wiki-test", "version": "0.1.0"}
        }
    }
    resp = send_recv(proc, req)
    print("Initialize:", json.dumps(
        resp.get("result", {}).get("serverInfo", {}), indent=2))
    # send initialized notification
    notif = {"jsonrpc": "2.0",
             "method": "notifications/initialized", "params": {}}
    body = json.dumps(notif)
    header = f"Content-Length: {len(body)}\r\n\r\n"
    proc.stdin.write(header.encode())
    proc.stdin.write(body.encode())
    proc.stdin.flush()


def main():
    env = os.environ.copy()
    env["GITNOVA_REPO"] = REPO
    proc = subprocess.Popen(
        CMD,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=env
    )

    try:
        initialize(proc)

        # 1. Create index page for ha3
        print("\n=== Creating ha3 index page ===")
        resp = call_tool(proc, "wiki_upsert_page", {
            "id": "ha3",
            "title": "Havenask (HA3) Architecture",
            "kind": "index",
            "summary": "Havenask is a large-scale C++ search engine with inverted index, query execution, and build pipeline subsystems.",
            "content": """# Havenask (HA3) Architecture

Havenask is a large-scale C++ search engine developed by Alibaba. It provides:
- **Inverted Index**: Full-text search with BM25 ranking
- **Query Execution**: Multi-stage query pipeline with term/phrase/bitmap executors
- **Build Pipeline**: Real-time and offline document indexing
- **KV/KKV Store**: Primary key lookup via hash table

## Key Subsystems
- [Search & Query](ha3/search) — query parsing, term execution, result ranking
- [Index Build](ha3/index-build) — inverted index construction, segment management
- [Storage](ha3/storage) — indexlib storage layer, segment readers/writers
- [Build Service](ha3/build-service) — distributed build administration
"""
        })
        print(json.dumps(resp, indent=2, ensure_ascii=False)
              if resp else "No response")

        # 2. Search & Query subsystem
        print("\n=== Creating search/query page ===")
        resp = call_tool(proc, "wiki_upsert_page", {
            "id": "ha3/search",
            "title": "Search & Query Execution",
            "kind": "article",
            "parent": "ha3",
            "summary": "Multi-stage query pipeline: parse → optimize → execute → rank",
            "content": """# Search & Query Execution

## Query Pipeline
1. **QueryParser** — parses search string into query tree
2. **QueryOptimizer** — rewrites query for performance
3. **TermQueryExecutor** — executes term-level search against posting lists
4. **BitmapTermQueryExecutor** — bitmap-based term matching
5. **SpatialTermQueryExecutor** — spatial search support
6. **ResultRanker** — BM25/TF-IDF scoring and sorting

## Key Classes
- `TermQueryExecutor` — base class for term execution
- `BitmapTermQueryExecutor` — bitmap intersection for AND queries
- `SubFieldMapTermQueryExecutor` — sub-field aware execution
- `InvertedIndexSearchTracer` — search performance tracing

## Posting List Format
- Skiplist-based compressed posting lists
- Section-weight encoding for positional information
"""
        })
        print(json.dumps(resp, indent=2, ensure_ascii=False)
              if resp else "No response")

        # 3. Index Build subsystem
        print("\n=== Creating index-build page ===")
        resp = call_tool(proc, "wiki_upsert_page", {
            "id": "ha3/index-build",
            "title": "Index Build Pipeline",
            "kind": "article",
            "parent": "ha3",
            "summary": "Real-time and offline document indexing with segment management",
            "content": """# Index Build Pipeline

## Build Architecture
- **InvertedIndexBuildWorkItem** — builds inverted index for a segment
- **BuildingIndexReader** — reads from in-progress building index
- **BuildWorkItem** — base class for all build work items

## Segment Lifecycle
1. **Building** — segment under construction (in-memory)
2. **Dump** — segment being persisted to disk
3. **Built** — segment fully built and searchable

## Key Classes
- `InvertedIndexBuildWorkItem extends BuildWorkItem`
- `BuildingDateIndexReader`, `BuildingRangeIndexReader`
- `SortDocumentContainer` — document sorting during build
"""
        })
        print(json.dumps(resp, indent=2, ensure_ascii=False)
              if resp else "No response")

        # 4. Storage subsystem
        print("\n=== Creating storage page ===")
        resp = call_tool(proc, "wiki_upsert_page", {
            "id": "ha3/storage",
            "title": "Storage Layer (indexlib)",
            "kind": "article",
            "parent": "ha3",
            "summary": "indexlib storage: hash table primary key, segment readers, KV/KKV",
            "content": """# Storage Layer (indexlib)

## Primary Key Lookup
- `HashTablePrimaryKeyDiskIndexer::Lookup` — on-disk hash table lookup
- `KKVReader::LookupAsync` — async KKV pkey lookup
- `HashTableVarSegmentReader::Get` — variable-length segment read
- `HashTableCompressVarSegmentReader::Get` — compressed variant

## Segment Reader Hierarchy
- `IndexSegmentReader` → `InMemorySegmentReader` / `OnDiskSegmentReader`
- `IndexPartitionReader` — unified reader across segments

## KV/KKV Store
- **KV**: single-value primary key store
- **KKV**: multi-value primary key store (prefix-compressed)
"""
        })
        print(json.dumps(resp, indent=2, ensure_ascii=False)
              if resp else "No response")

        # 5. Add evidence to search page
        print("\n=== Adding evidence to search page ===")
        resp = call_tool(proc, "wiki_append_evidence", {
            "id": "ha3/search",
            "file": "aios/storage/indexlib/index/inverted_index/InvertedIndexBuildWorkItem.h",
            "start_line": 1,
            "end_line": 50,
            "note": "InvertedIndexBuildWorkItem class definition"
        })
        print(json.dumps(resp, indent=2, ensure_ascii=False)
              if resp else "No response")

        # 6. Read back a page
        print("\n=== Reading ha3 index page ===")
        resp = call_tool(proc, "wiki_read_page", {
            "id": "ha3",
            "include_private": False
        })
        if resp and "result" in resp:
            content = resp["result"].get("content", [])
            for item in content:
                if isinstance(item, dict) and "text" in item:
                    print(item["text"][:500])
        else:
            print(json.dumps(resp, indent=2, ensure_ascii=False)
                  if resp else "No response")

    finally:
        proc.terminate()
        proc.wait()


if __name__ == "__main__":
    main()
