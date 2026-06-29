# Reserve Accounting

The `withdraw_reserve` entrypoint routes accrued interest to the protocol treasury safely.

### Security Boundaries
- **Admin-Gated:** Protected by explicit `assert_admin`.
- **Principal Isolation:** amount is bounded by the accumulator to protect depositors.
- **State Emissions:** Emits a `ReserveWithdrawn` event.