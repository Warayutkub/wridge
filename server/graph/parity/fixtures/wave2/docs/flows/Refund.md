# Refund
entry: admin

## Steps
1) [admin] clicks refund `GET /api/refund`
2. [server] `refundPayment` checks `payments.refunded_at`
3. [⏳ stripe] `stripe` webhook
