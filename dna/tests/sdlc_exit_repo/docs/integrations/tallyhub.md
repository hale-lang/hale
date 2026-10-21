# Tallyhub integration

Tallyhub is the ledger service the shipyard posts entries to. The client in
tallyhub/client.go builds the form body of an entry from an account and an
amount in cents, and sends nothing else: no memo, no currency. A change to
Tallyhub's changelog matters only if it makes one of those two fields
mean something else, or requires a field the client does not send.
