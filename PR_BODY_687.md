## Description
This PR addresses issue #687 by adding deterministic withdraw constraint tests under collateral ratio boundaries. It ensures that withdrawals which maintain a healthy position succeed, while those violating requirements fail with stable errors.

## Key Changes
- **New Test Suite**: Added `withdraw_boundary_test.rs` covering single-asset, multi-asset, and price-drop scenarios precisely at the 1.0 Health Factor boundary.
- **Rounding Precision**: Added tests for "rounding dust" to ensure precision at the edge of collateralization.
- **Protocol Refactor**: Updated `cross_asset.rs` to use the real `oracle` module instead of a hardcoded mock, enabling realistic price-move testing.
- **Documentation**: Updated `CROSS_ASSET_RULES.md` with detailed withdrawal boundary examples and security notes.

## Verification
- Ran all 4 boundary tests: all PASSED.
- Verified Health Factor calculations at thresholds (HF=1.0).
- Verified failure modes for undercollateralized withdrawal attempts.

Closes #687
