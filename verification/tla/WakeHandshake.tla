--------------------------- MODULE WakeHandshake ---------------------------
EXTENDS Integers, Sequences, FiniteSets

(* Mailbox/classic-pool wake protocol, after the C11 visibility obligation
   checked by mailbox_model.c / coop_pool_model.c. One consumer; each producer
   publishes at most one cell into a ring large enough for all producers.
   The mutex covers announcement/recheck/cond_wait and signal/broadcast.
   Signal has no durable credit: signalling before a wait loses that signal.
   See README.md for source correspondence, exclusions and fairness. *)

CONSTANTS NumProducers, EnableShutdown, OmitRecheck, DropSignal, UnprotectedSignal
Producers == 1..NumProducers
VARIABLES queue, published, consumed, producer, consumer, parked, mutex,
          shutdown, stopping
vars == <<queue, published, consumed, producer, consumer, parked, mutex,
          shutdown, stopping>>

Init == /\ queue = <<>> /\ published = {} /\ consumed = {}
        /\ producer = [p \in Producers |-> "post"]
        /\ consumer = "try" /\ parked = FALSE /\ mutex = 0
        /\ shutdown = FALSE /\ stopping = "request"

Producer(p) ==
  /\ UNCHANGED <<consumed, parked, shutdown, stopping>>
  /\ \/ /\ producer[p] = "post"
        /\ IF shutdown
              THEN /\ producer' = [producer EXCEPT ![p] = "done"]
                   /\ UNCHANGED <<queue, published>>
              ELSE /\ queue' = Append(queue, p)
                   /\ published' = published \cup {p}
                   /\ producer' = [producer EXCEPT ![p] = "observe"]
        /\ UNCHANGED <<consumer, mutex>>
     \/ /\ producer[p] = "observe"
        /\ producer' = [producer EXCEPT ![p] =
             IF ~parked \/ DropSignal THEN "done"
             ELSE IF UnprotectedSignal THEN "signal" ELSE "lock"]
        /\ UNCHANGED <<queue, published, consumer, mutex>>
     \/ /\ producer[p] = "lock" /\ mutex = 0
        /\ mutex' = p /\ producer' = [producer EXCEPT ![p] = "signal"]
        /\ UNCHANGED <<queue, published, consumer>>
     \/ /\ producer[p] = "signal"
        /\ consumer' = IF consumer = "asleep" THEN "reacquire" ELSE consumer
        /\ producer' = [producer EXCEPT ![p] =
             IF UnprotectedSignal THEN "done" ELSE "unlock"]
        /\ UNCHANGED <<queue, published, mutex>>
     \/ /\ producer[p] = "unlock" /\ mutex = p
        /\ mutex' = 0 /\ producer' = [producer EXCEPT ![p] = "done"]
        /\ UNCHANGED <<queue, published, consumer>>

Consumer ==
  /\ UNCHANGED <<published, producer, shutdown, stopping>>
  /\ \/ /\ consumer = "try"
        /\ IF Len(queue) > 0
              THEN /\ consumed' = consumed \cup {Head(queue)}
                   /\ queue' = Tail(queue)
                   /\ UNCHANGED <<consumer, parked, mutex>>
              ELSE /\ consumer' = "lock"
                   /\ UNCHANGED <<queue, consumed, parked, mutex>>
     \/ /\ consumer \in {"lock", "reacquire"} /\ mutex = 0
        /\ mutex' = -1
        /\ consumer' = IF consumer = "lock" THEN "announce" ELSE "clear"
        /\ UNCHANGED <<queue, consumed, parked>>
     \/ /\ consumer = "announce" /\ mutex = -1
        /\ parked' = TRUE /\ consumer' = "recheck"
        /\ UNCHANGED <<queue, consumed, mutex>>
     \/ /\ consumer = "recheck" /\ mutex = -1
        /\ IF ~OmitRecheck /\ Len(queue) > 0
              THEN /\ consumed' = consumed \cup {Head(queue)}
                   /\ queue' = Tail(queue) /\ consumer' = "clear"
              ELSE /\ consumer' = IF shutdown THEN "exit" ELSE "wait"
                   /\ UNCHANGED <<queue, consumed>>
        /\ UNCHANGED <<parked, mutex>>
     \/ /\ consumer = "wait" /\ mutex = -1
        (* pthread_cond_wait atomically registers the waiter and unlocks. *)
        /\ consumer' = "asleep" /\ mutex' = 0
        /\ UNCHANGED <<queue, consumed, parked>>
     \/ /\ consumer = "clear" /\ mutex = -1
        /\ parked' = FALSE /\ consumer' = "unlock"
        /\ UNCHANGED <<queue, consumed, mutex>>
     \/ /\ consumer = "unlock" /\ mutex = -1
        /\ mutex' = 0 /\ consumer' = "try"
        /\ UNCHANGED <<queue, consumed, parked>>
     \/ /\ consumer = "exit" /\ mutex = -1
        /\ mutex' = 0 /\ parked' = FALSE /\ consumer' = "done"
        /\ UNCHANGED <<queue, consumed>>

Stop ==
  /\ EnableShutdown
  /\ UNCHANGED <<queue, published, consumed, producer, parked>>
  /\ \/ /\ stopping = "request" /\ mutex = 0
        (* The lifecycle caller quiesces ingress before shutdown. *)
        /\ \A p \in Producers: producer[p] = "done"
        /\ mutex' = -2 /\ shutdown' = TRUE /\ stopping' = "unlock"
        /\ consumer' = IF consumer = "asleep" THEN "reacquire" ELSE consumer
     \/ /\ stopping = "unlock" /\ mutex = -2
        /\ mutex' = 0 /\ stopping' = "done"
        /\ UNCHANGED <<shutdown, consumer>>

(* POSIX permits spurious wakeups, but does not promise to supply them. *)
SpuriousWake == /\ consumer = "asleep" /\ consumer' = "reacquire"
                /\ UNCHANGED <<queue, published, consumed, producer, parked,
                               mutex, shutdown, stopping>>
Next == (\E p \in Producers: Producer(p)) \/ Consumer \/ Stop \/ SpuriousWake
Spec == Init /\ [][Next]_vars
        /\ (\A p \in Producers: WF_vars(Producer(p)))
        /\ WF_vars(Consumer) /\ WF_vars(Stop)

Queued == {queue[i] : i \in 1..Len(queue)}
TypeOK == /\ queue \in Seq(Producers) /\ Len(queue) <= NumProducers
          /\ published \subseteq Producers /\ consumed \subseteq Producers
          /\ producer \in [Producers -> {"post", "observe", "lock", "signal", "unlock", "done"}]
          /\ consumer \in {"try", "lock", "announce", "recheck", "wait", "asleep",
                           "reacquire", "clear", "unlock", "exit", "done"}
          /\ parked \in BOOLEAN /\ shutdown \in BOOLEAN
          /\ mutex \in Producers \cup {0, -1, -2}
          /\ stopping \in {"request", "unlock", "done"}
Accounted == /\ published = consumed \cup Queued
             /\ consumed \intersect Queued = {}
             /\ Cardinality(published) = Cardinality(consumed) + Len(queue)
ExitDrained == consumer = "done" => shutdown /\ queue = <<>> /\ ~parked
AcceptedEventuallyConsumed == \A p \in Producers: (p \in published) ~> (p \in consumed)
ShutdownEventuallyExits == shutdown ~> (consumer = "done")
=============================================================================
