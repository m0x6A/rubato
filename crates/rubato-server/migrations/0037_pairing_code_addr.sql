-- ADR 0090 (plan 345, design §7): a pending pairing code remembers the address that asked for
-- it, so the codes one address may hold are capped as well as the server's. NULL for a code
-- from before this migration, and where the server does not know its peer.
ALTER TABLE pairing_codes ADD COLUMN client_addr TEXT;
