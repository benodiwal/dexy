# Dexy

A Solana-based automated market maker (AMM) program built with the Anchor framework.

## Overview

Dexy implements a decentralized exchange with constant product curve functionality, allowing users to swap tokens through an automated market maker protocol.

## Features

- **Token Swapping**: Execute token swaps using automated market maker logic
- **Liquidity Management**: Initialize and manage liquidity pools
- **Constant Product Curve**: Implements the x*y=k pricing model
- **Fee Structure**: Configurable trading fees
- **Safety Checks**: Built-in overflow protection and validation

## Development

This project uses the Anchor framework for Solana program development.

### Prerequisites

- Rust
- Solana CLI
- Anchor CLI

### Build

```bash
anchor build
```

### Test

```bash
anchor test
```

## Program Structure

- `programs/dexy/` - Main program implementation
- `curve/` - AMM curve calculations and fee logic
- Core functions include `initialize` for pool setup and `swap` for token exchanges

## License

This project is built with Anchor framework.