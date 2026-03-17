# Kind2 v3.0.0
FROM kind2/kind2@sha256:b5b7c7fde98754040ea99385c83afc9339e6a244732aa2ad0f6ccf0cd517465a

WORKDIR /work

COPY proofs/basic_recurrence.lus /work/proofs/basic_recurrence.lus

ENTRYPOINT ["/kind2"]
CMD ["--lus_main", "main", "/work/proofs/basic_recurrence.lus"]
