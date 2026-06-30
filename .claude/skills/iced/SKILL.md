---
name: iced
description: "Use when building GUIs with the Iced Rust framework (Elm architecture: Message/update/view)."
---

# Iced (Rust GUI)

Iced follows the Elm architecture: state + `Message` + `update` + `view`.

## Model
- State lives in your app struct. Define a `Message` enum for every event.
- `update(&mut self, message) -> Task<Message>`: mutate state; return a `Task` for side effects (async, I/O) — never block in `update`/`view`.
- `view(&self) -> Element<Message>`: pure; build from widgets (`column!`, `row!`, `text`, `button`, `text_input`, `container`, `scrollable`).
- `subscription(&self)`: for timers, streams, external events.

## Tips
- Buttons/inputs emit messages via `.on_press(Message::…)` / `.on_input(Message::…)`.
- Compose child components and map their messages with `.map(Message::Child)`.
- Use `Theme`/styling functions; keep layout in `view`, logic in `update`.
- Long work → spawn via `Task::perform(future, Message::Done)`.

## Verify the UI against the design (iced-test)
Don't just eyeball it — render and check. Use the `iced_test` crate to drive a
`view`/`update` headlessly and **render a screenshot**, then compare it to the design.
- Build a `iced_test` simulator for your element/app, simulate any needed interactions, and
  capture the rendered output to a PNG.
- Compare the screenshot against the intended design (e.g. the mockup referenced in the ba
  doc for this component/todo). Note layout, spacing, colors, and states (hover/active/empty).
- Keep these as snapshot tests so regressions are caught: re-render on change and diff vs the
  approved screenshot; update the baseline only when the design intentionally changes.
- In your report, state that you verified the rendered screenshot matches the design (and call
  out any deliberate deviations).
