# Orders API: price cache (LLD)

## Container change

Checkout prices every cart line from the database today. This change puts
a cache in front of it.

### Before

```mermaid
flowchart LR
  web["Web app [Container: TypeScript]<br/>pages, cart"]
  subgraph shop["Shop [Software System]"]
    api["Orders API [Container: Rust]<br/>orders, stock, checkout"]
    db[("Orders DB [Container: PostgreSQL]<br/>orders and stock")]
    report["Reports [Container]<br/>nightly sales export"]
  end
  web -->|JSON over HTTPS| api
  api -->|reads prices| db
  report -->|reads| db
```

### After

```mermaid
flowchart LR
  web["Web app [Container: TypeScript]<br/>pages, cart"]
  subgraph shop["Shop [Software System]"]
    api["Orders API [Container: Rust]<br/>orders, stock, checkout, cached prices"]
    cache[("Price cache [Container: Redis]<br/>prices for 60 s")]
    db[("Orders DB [Container: PostgreSQL]<br/>orders and stock")]
  end
  web -->|JSON over HTTPS| api
  api -->|reads prices| cache
  cache -->|fills from| db
  api -.->|reads prices| db
  classDef new stroke-dasharray: 4 2
  classDef changed stroke-width:2px
  class cache new
  class api changed
```

## Storage

### 1. Current

```mermaid
flowchart TB
  db[("Orders DB [Container: PostgreSQL]<br/>orders and stock")]
```

### 2. Proposed: a replica

```mermaid
flowchart TB
  db[("Orders DB [Container: PostgreSQL]<br/>orders and stock")]
  replica[("Read replica [Container: PostgreSQL]<br/>reports read here")]
  db -->|streams WAL| replica
```

## Notes

### Before we start

Prose only: a heading starting with "Before" whose section has no
diagram doesn't pair.
