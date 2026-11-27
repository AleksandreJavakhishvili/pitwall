# Diagrams the tile doesn't draw

## Class

```mermaid
classDiagram
  class Order {
    +id: Uuid
    +total() Money
  }
  Order "1" --> "*" Line
```

## State

```mermaid
stateDiagram-v2
  [*] --> Draft
  Draft --> Paid: pay
  Paid --> [*]
```

## Entity relationship

```mermaid
erDiagram
  ORDER ||--o{ LINE : contains
```

## Gantt

```mermaid
gantt
  title Release
  dateFormat YYYY-MM-DD
  section Build
  Parser :a1, 2026-11-27, 3d
```

## Pie

```mermaid
pie title Tests
  "pass" : 120
  "fail" : 3
```

## Mindmap

```mermaid
mindmap
  root((C4 tile))
    Parser
    Layout
```

## Timeline

```mermaid
timeline
  title Tile SDK
  2026-11 : Phase 0
  2026-12 : Phase 1
```

## Mermaid's own C4

```mermaid
C4Context
  title System Context diagram for the shop
  Person(customer, "Customer", "buys things")
  System(shop, "Shop", "sells things")
  Rel(customer, shop, "Uses")
```

## Unknown word

```mermaid
zenDiagram
  a -> b
```

## Empty

```mermaid
%% only a comment
```
