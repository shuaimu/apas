## 1. Decide "behind" once

- [x] 1.1 Add a version helper in `packages/web/src/lib/` that parses `YY.MM.COMMIT` into a comparable triple and returns unknown for anything else, matching the CLI's ordering rather than approximating it
- [x] 1.2 Add the "latest seen" reduction over the server version and the reachable machines' daemon versions, excluding unparseable ones from the maximum
- [x] 1.3 Expose one predicate the surfaces call — is this machine behind — so neither computes it itself
- [x] 1.4 Tests: older than a peer, older than the server, newest machine, newer than the server, missing version, unparseable version, and a pair whose text ordering disagrees with release ordering (e.g. `26.08.9` against `26.08.10`)
- [x] 1.5 Test that an unparseable version cannot mark its peers behind by distorting the maximum

## 2. Mobile machine list

- [x] 2.1 Show each machine's reported daemon version in its row, saying so plainly when it is unknown
- [x] 2.2 Label the existing reboot control from the predicate: updating wording when behind, plain otherwise
- [x] 2.3 Carry the same wording into the confirmation sheet, which already names the machine
- [x] 2.4 Tests: both labels render for the right machines, an unknown version gets the plain label, and confirming either wording sends the same request for that machine

## 3. Desktop machines page

- [x] 3.1 Add the daemon restart control to each machine row on `/machines`, which has none today
- [x] 3.2 Add its confirmation, identifying the machine, matching the mobile behaviour
- [x] 3.3 Show the reported daemon version on the machine row
- [x] 3.4 Apply the same label from the same predicate
- [x] 3.5 Tests: the control appears per machine, confirming sends the restart for that machine, dismissing sends nothing, and the label matches what mobile shows for the same machine

## 4. Verification

- [x] 4.1 `npm run lint` and the web test suite clean
- [x] 4.2 Check the two surfaces against one another on the same account: same machines, same versions, same wording — done as a test that renders both against identical machine data and compares the control names, rather than left as a convention two independently written components could drift from
- [ ] 4.3 Live check on the real cluster: with the hosts level, every machine reads plain; after installing a newer CLI on one host, the hosts that have not yet taken it read as updating, and lose that wording once their own upgrade tick lands
