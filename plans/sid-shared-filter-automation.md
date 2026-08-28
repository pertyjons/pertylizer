# Shared return-bus automation for SID filters

> **Status:** proposed.
>
> **Planning baseline:** Pertylizer `3cfb9581`, 2026-08-27. The checkout also
> contains unrelated in-progress Core V2 transport changes; this plan does not
> depend on or modify them.

## Objective

Make one shared return-bus Filter reproduce a SID chip's global filter when its
voice routing, cutoff, and resonance change over time. The same saved project
must behave identically in live playback and the headless/offline renderer, and
the automation must be authorable and discoverable through MCP.

This closes the concrete sid-analyzer fallback where static routing can use one
true return filter, but an in-track route change or shared cutoff sweep requires
duplicated per-instrument filters.

## Confirmed gap

- `synth_sequencer::AutomationTarget` has `Instrument`, `Module`, `Track`, and
  `Global` variants only.
- `TrackSend`, `ReturnBus`, and `ReturnSend` persist static levels and faders in
  `synth_sequencer/src/track.rs`.
- Return effects persist separately through
  `GlobalProjectState::return_bus_effects`.
- `EngineCommand::SetReturnEffectParameter` already changes a live return
  effect, but sequencer `Parameter` events dispatch only to instrument-owned
  modules or global master volume.
- `Filter` marks continuous `cutoff` and `resonance` parameters automatable;
  its `type` and `model` choices are deliberately non-automatable.
- `plans/TODO.md` sections 1.3 and 2.3 already record the routing and
  master/return-effect halves of the gap. They must land together for the SID
  use case; either half alone still forces a fallback.

## Scope decisions

Add these target concepts without changing the meaning of an existing target:

```rust
AutomationTarget::TrackSend {
    track: Option<TrackId>,
    return_bus: ReturnBusId,
}
AutomationTarget::Return {
    return_bus: ReturnBusId,
    param: ReturnParam,
}
AutomationTarget::ReturnEffect {
    return_bus: ReturnBusId,
    module_type: ModuleType,
    instance: u16,
    param_id: ParamId,
}
```

`ReturnParam` initially contains `Volume` and `Mute`. Track-send automation
controls the level of an already-authored send; it does not create routing on
the audio thread. `track: None` has the same host-track resolution semantics as
the existing relative `Track` target.

Return-effect identity is `(ReturnBusId, ModuleType, instance)`, matching the
persisted module ID. Do not use chain position or display name as identity.
Parameter values remain normalized in lanes and are denormalized from the live
effect descriptor by `param_id`, as instrument `Module` automation is today.

This plan does not make choice parameters automatable. SID filter-mode changes
need a separate step/discrete-automation contract; silently mapping a continuous
lane onto `FilterMode` would change the current descriptor meaning.

## Runtime design

### Sequencer state

Extend the existing transport-reset-owned automation override state with
pre-keyed, bounded routing controls:

- track-send override keyed by `(TrackId, ReturnBusId)`;
- return-fader override keyed by `ReturnBusId`;
- return-effect parameter events retained as `SequencerEvent::Parameter`.

Keys and capacity are prepared when the song/runtime snapshot is rebuilt.
Playback must not allocate, grow a map, acquire a lock, or construct an
interned parameter ID. Removing a lane or resetting transport returns the
target to its persisted static value in the same block.

### Mix application

In `SynthEngine::update_track_controls`, replace a static `TrackSend::level`
with the matching override when present before creating the bounded
`ChannelSend`. Preserve `enabled` and `pre_fader` semantics.

Apply return `Volume`/`Mute` overrides while copying each song `ReturnBus`
fader into `ReturnBusChannel`. Do not mutate the song from the audio thread.

Dispatch `ReturnEffect` events against `self.return_busses` before processing
the return chain. Resolve the effect by return ID and module ID, resolve the
descriptor parameter by `param_id`, denormalize it, and use the effect's
transient override layer so transport stop restores the persisted base value.
If `EffectChain` lacks the same override API as instrument modules, add it once
there rather than writing a Filter-specific branch in `SynthEngine`.

The engine-side shared snapshots represent persisted base parameters. Playback
automation must not rewrite `return_bus_effects`, or saving during playback
would bake an arbitrary lane sample into the project.

## Authoring and validation

Extend the MCP target DSL and structured target forms with:

```text
track:Send:<return_id>
track:<track_id>:Send:<return_id>
return:<return_id>:Volume
return:<return_id>:Mute
return:<return_id>:module:<short-key>-<instance>:<param_id>
```

Exact spelling may follow the existing parser grammar, but parse/format must
round-trip one canonical string. Validate before lane mutation that:

- the concrete track exists when supplied;
- the return bus exists;
- the track already has the addressed send;
- the return effect exists on that bus;
- the parameter exists in its live descriptor and `is_automatable()` is true.

Expose all three target types through automation discovery and lane listing.
Errors name the missing track, bus, send, effect, or parameter rather than
falling through to an instrument-module diagnostic.

## Persistence contract

The new enum variants are additive: old version-1.1 projects retain their
meaning and load unchanged. Before implementation, confirm whether the project
schema policy requires a format-version bump for an expanded enum even though
old data remains valid. Do not silently reinterpret an old serialized target.

Round-trip each target with `serde` and the generated schema. Deleting a track
or return bus must strip every lane that concretely targets it. A relative
host-track send lane survives track deletion with its owning pattern, matching
existing host-track automation semantics.

## Implementation phases

1. Add the sequencer target types, resolution, display names, strict serde
   tests, deletion cleanup, and canonical target-string round-trip.
2. Add bounded runtime override storage and reset semantics for track sends and
   return faders.
3. Add generic transient parameter overrides to return/master effect chains and
   dispatch `ReturnEffect` events.
4. Extend MCP construction, validation, discovery, lane read/write operations,
   and schemas.
5. Prove live/headless parity with a shared-filter fixture before removing the
   TODO entries.

Do not land a target variant before its runtime application and validation are
present; a serialized lane that silently does nothing is a contract defect.

## Verification

Add positive and negative controls for:

- one track's send stepping `0 → 1 → 0` while another track remains routed;
- host-relative and explicit-track send targets;
- return volume and mute reset to persisted values after transport stop;
- return Filter cutoff and resonance ramps reaching the same normalized values
  in live and headless renders;
- two effects of the same module type on different returns resolving by stable
  bus/module identity;
- missing bus, missing authored send, missing effect, non-automatable `type`,
  and unknown parameter rejection;
- save during playback retaining base effect parameters;
- track/return deletion removing only the lanes whose concrete owner vanished;
- project/schema and MCP target-string round trips.

The audio-path test must run after capacity has been prepared and assert zero
allocation/lock activity through the repository's existing real-time checks.

## Exit gate

- A sid-analyzer project with one return Filter can sweep cutoff/resonance and
  change which SID voice is routed without duplicating the Filter.
- Live playback and the headless render are sample-identical for the synthetic
  routing/sweep fixture.
- Stopping/restarting transport restores every persisted send, return fader,
  and return-effect base value.
- MCP discovers, validates, creates, reads, and round-trips every new target.
- Invalid targets fail before mutation and no supported lane is silently
  ignored.
- The Pertylizer feature gate and independent review required by `AGENTS.md`
  pass before the implementation is committed.
