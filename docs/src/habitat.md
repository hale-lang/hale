# The habitat, as designed

> **This chapter is a design, not a description of the tree.** It collects the direction settled in two design reviews:
> - the habitat RFC (GH #602), delivered piece by piece and continued in the organism's map (GH #990);
> - the authority note and final direction of the compiler RFC (GH #1212).
>
> The last section lists which pieces exist today, each with a link. Anything not in that list is not built.

## A living habitat for people and software

An organism has identity, responsibilities, memory, resources, procedures and a lifecycle. Its activity maintains a **habitat**: the shared records, tools, channels and practices the work runs in. The two are perspectives, not kinds of thing. One effort's organism can be the habitat of several others, and the relationships between them need not form a tree.

The promise to a person is: start around the work you are responsible for, connect the people and services involved, and grow a system that carries work through to evidenced outcomes while it improves its own practices and software.

That holds at every scale:
- an individual contributor, for their own responsibilities inside an employer;
- a lead for a team, or a director across teams;
- a founder for a venture;
- firms cooperating on one piece of work.

It must never require incorporation, control of an employer's systems, or a new accounting platform. Existing reporting and approval channels stay usable.

A habitat is not a new compiler primitive, a tenant or a legal entity. The org chart is one view of it among several.

## Records are the unit of isolation

The unit of isolation is **a repository and its record**, with the backends attached to it (memory, nerves, secrets, the senses), scoped to that record.
- One person can take part in several records, and several people can share one.
- A clone is a replica of a record, not a new access domain.
- Moving or cloning a record keeps its identity and provenance.

**Sharing is deliberate publication.** Two firms keep their private records and agree on a third, shared one. Into it they publish selected facts, each carrying its origin, lineage and purpose. Syncing a whole record cannot export part of its private history, so a shared record never receives one. Sharing participants, owners or grants does not merge private context, administration or books.

**Visibility is enforced, not labelled.** Whoever holds a full replica can read its plaintext, and a projection that filters rows cannot hide them. Sensitive material therefore lives outside the replicated record, as protected evidence in the record's own memory, read only through a principal-checked boundary. This covers the derivatives of sensitive material too: summaries, embeddings, prompts and the context a worker is given.

**Identity is bound at every entry**: the face, the API, the command line, a synced decision, a callback, resumed work. Each checks the actor, the record, the current membership or assignment, and the policy. A git author, a `--as` flag or a position in a view is attribution, not proof.

## Three independent choices

| axis | choices |
| --- | --- |
| participation | one person; a team on one record; firms cooperating through a shared record |
| deployment | on your machine; hosted behind a domain; other admitted placements |
| principal source | explicit local trust; a verified identity provider (OIDC) |

Each axis is chosen independently of the others. A hosted single user with an identity provider is a normal first profile.

## Authority is part of the graph

Today the compiler knows who owns a locus (the tower) and what a locus can reach (effects). It does not know on whose authority a locus runs. So "may this code reach this secret, this money, this change to the org chart" is answered at run time by whoever holds a token.

The design adds one relation to the tower: `runs_under(locus, principal)`. A locus declares the principal it runs under, and a locus with no declaration inherits its owner's. The compiler closes the relation the way it closes ownership: no locus may run under a principal its owner does not hold. Principals are declared by the program, and a DNA position is the organism's instance of one. Three things follow, with no new kind of declaration:

1. **Claims cover the static half.** Reaching a secret slot becomes an effect with a principal attached. "Only a reviewer's code reaches the forge token" is then the same shape of claim as "no model-driven code reaches money except through authority" ([Claims & the law](./claims.md)), checked at build.
2. **Backend policies are projections, not authored files.** A vault policy, a database role, a broker account's permissions and a forge team are each the reachable closure of one principal, rendered in one backend's syntax when the program is placed. Nobody writes them by hand, and a hand edit is not a change. Which backend answers a secret slot is a choice made at placement, never the program's: a file directory on your machine, a vault with a role per principal in a habitat, a cloud engine.
3. **The run keeps the dynamic half, in the record.** Who holds a seat is an appointment row. Who is acting for that holder right now is an **attachment**: one worker process or one face session, under one seat, for a bounded time, with a scope inside the seat's closure. A credential is cut for the attachment, never for the seat. It is short-lived, narrowed to the attachment's scope, and revoked when the attachment ends. The compiler proves the ceiling, the run enforces the narrowing, and every effect is recorded against its attachment.

## Responsibilities grow; equipment equips them

A new responsibility is a child the organism proposes from evidence of a need and the authority admits, through the organism's existing lifecycle. There is no preinstalled department taxonomy. **Standard equipment** is libraries that give a responsibility competence:
- research and decision briefs;
- qualification and offers;
- implementation and release;
- onboarding and support flows;
- receipt capture and reconciliation;
- publication and experiments;
- access and documents;
- deployment and recovery.

Installing equipment creates no department, appoints no performer and grants no authority.

Each equipped responsibility declares:
- the outcomes it owns, and the work it accepts;
- its procedures, and what its performers must be able to do;
- its mandates and budgets;
- what it counts as acceptance, and how it is evaluated;
- how it recovers.

A request that crosses scopes follows a declared contract. Sales cannot declare delivery accepted, and assigning a proposal to yourself does not create authority to change it.

## Money from the first receipt

Finance is a responsibility from the start, even when its performer is a person and it only captures.
- **One chain of distinct steps.** Source evidence becomes observations, observations become economic events, and those become approved postings or external acknowledgements, each a separate step. A paid bill may stay unclassified, and a posted invoice may stay unpaid.
- **One authority for each book.** It is capture only, the organism's own posting service, or an external platform that owns the book and its close.
- **Money is exact.** Amounts are `Decimal` with an explicit currency and rounding policy, never binary floating point.
- **Budgets keep five things apart:** the permitted budget, reserved capacity, observed usage, the booked expense and cash settlement. A reservation is not an expense, and a bill and the card payment for it are not two expenses.

No automated payment is part of the first slice.

## Providers are replaceable

Models, stores, identity, communications, payments, accounting and hosting are bound through interfaces and perspectives, under the same claims and ownership rules as everything else ([Perspectives](./services/perspectives.md)). A provider's capability is kept in three separate states: implemented, available on this account, and authorized. A feature appearing at a provider never widens a grant. Replacing a provider keeps the organization's intent, authority, records and history, although mandates and pending operations may stay bound to the original service until they are reconciled.

## What the tree has today

| piece of the design | in the tree today |
| --- | --- |
| a record per repository, never rewound | yes: [Memory and the record](./dna/memory.md) |
| deliberate publication between records | connections and handoffs of single facts with origin, lineage and purpose (`hale dna connect`, `hale dna handoff`): [Memory and the record](./dna/memory.md) |
| protected evidence outside the replicated record | receipts filed as `customer` or `confidential` are kept in memory alone, and every read is a row (`hale dna receipt`): [Memory and the record](./dna/memory.md) |
| deployment profiles | `hale dna new --profile local \| remote-body`, `hale dna profile`: [Getting started](./dna/getting-started.md) |
| a verified principal source | an OIDC-signed head (`git config dna.principal oidc`): [The head and the face](./dna/head.md), [The skin](./dna/skin.md) |
| roles per client on each backend | memory's roles, one broker account per role and per attached application, and vault slots: [The skin](./dna/skin.md) |
| money reserved against grants | spend reserved, settled and compensated under a grant's windows: [The spine](./dna/spine.md) |
| `runs_under`, policies rendered as projections, per-attachment credentials | **not built**: reserved as a relation for the compiler's registry (GH #1212) |
| standard equipment | **not built** |
| finance capture and books | **not built** |
