# Future Integrations

The charting engine should remain data-source agnostic.

One intended future integration is an existing C++17 limit order matching engine that exposes market data through gRPC/protobuf streaming APIs. That system includes:

- Price-time priority limit order book.
- Live bid/ask and Level 2 market depth streams.
- Order submission and cancellation APIs.
- Mean-reversion simulated trading client.
- Python FastAPI bridge and React frontend prototype.

This project should not depend on that system directly.

Instead, the charting engine should define neutral market data types and ingestion interfaces that could later be adapted to:

- gRPC market data streams.
- Exchange WebSocket streams.
- Historical files.
- Synthetic data generators.
- Backtest/replay engines.

The first MVP should use only synthetic data. The matching engine integration is a future adapter, not a core dependency.