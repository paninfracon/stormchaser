-- Deliveries a webhook has already accepted, so the same one is not acted on twice.
--
-- A delivery is authentic for as long as its signature is, which means anyone who
-- captures one can send it again and start the same workflows again. The API records a
-- key for each delivery it accepts (a hash of the signed body for GitHub, the signed
-- message id for signed webhooks) and ignores a delivery whose key it has seen within
-- the retention window.
CREATE TABLE webhook_deliveries (
    webhook_id UUID NOT NULL REFERENCES webhooks(id) ON DELETE CASCADE,
    delivery_key TEXT NOT NULL,
    received_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (webhook_id, delivery_key)
);

CREATE INDEX idx_webhook_deliveries_received_at ON webhook_deliveries(received_at);
