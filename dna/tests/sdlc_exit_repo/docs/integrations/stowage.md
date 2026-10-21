# Stowage integration

Stowage is the parcel service the shipyard registers cargo with. The client
in stowage/client.go marshals a NewParcel into the body of POST /parcels, and
stowage/client_test.go pins its shape. Every field Stowage's OpenAPI document
lists as required in the NewParcel schema must be a field of NewParcel, with
the document's name for it as its json tag.
