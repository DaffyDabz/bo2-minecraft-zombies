# Performance work

Frame-time work done for this fork and offered upstream as
[vladtrc/iw4L#8](https://github.com/vladtrc/iw4L/pull/8). Everything below is
included here.


## Before / after (no bots)

`mp_boneyard`, spawned as assault, holding forward while turning for 15 s. 1920x1080 windowed, vsync off (`IW4L_PRESENT_MODE=AutoNoVsync`), `IW4L_BENCH=1`. RTX 5090 + Ryzen 7 9800X3D, Windows 11. Five runs each, alternating base/PR so thermals and background load hit both equally. Medians:

| | master (`a6fa0b9`) | this PR | |
|---|---:|---:|---:|
| **avg fps** | **183** | **362** | **1.98x** |
| frame avg | 5.46 ms | 2.76 ms | -49% |
| frame p50 | 5.11 ms | 2.56 ms | |
| frame p95 | 8.26 ms | 4.00 ms | |
| frame p99 | 9.18 ms | 5.37 ms | |
| main thread `Update` | 4.22 ms | 2.26 ms | |
| render thread | 4.86 ms | 2.16 ms | |

Per run, base was 128, 183, 183, 181, 188 fps and the PR was 341, 358, 367, 362, 363 fps. The first run of each was a cold start; dropping it doesn't change either median.

The base binary is master plus only the one-line bench fix below, because on Windows master never writes the bench report.

The GPU sits at about 0.3 ms per frame through all of this, so the game is entirely CPU-bound. Every change below takes work off the main thread or the render thread, or splits it across cores. Neither thread is dominated by one thing any more: both are now roughly 2.2 ms spread over many systems.

## What changed

**Render thread: colour prepare / submit**
- **Camera colour rows are prepared on 4 lanes** (`ComputeTaskPool` scope). Each lane takes a contiguous share of the ordered rows and keeps its own material executor, pack cache and constant arena between frames. Lanes are concatenated in lane order, so draw order is unchanged. The submit loop switches bind group 0 when the lane changes. The scene texture tables are shared between lanes behind a mutex.
- **Constant arena uploads write straight into staging.** They no longer build an intermediate `Vec` first.
- **Glass panes outside the camera view are culled** before material execution. Glass was one draw per pane across the whole map and reached prepare unculled. Each glass draw now carries a bounding sphere.
- **CPU-skinned static model surfaces are kept across frames.** They are keyed on the installed static geometry, with incremental uploads of newly seen surfaces only. The recorder now takes cloned buffer handles instead of the caches.
- **Packed code constants are hashed a row at a time.** This uses a full 64x64→128 folded multiply, not a byte-at-a-time hash. Hash strength is unchanged, since the id is still used without a second comparison.

**Model lighting**
- **Per-frame tile updates no longer re-create the atlas texture.** The cache writes the atlas image untracked and queues the changed tiles in `ModelLightingAtlasTileWrites`, which are extracted and uploaded as 4x4x4 `write_texture` regions. Before, any moving model re-extracted and re-created the whole 3D texture every frame.
- Owner tables use foldhash (`bevy::platform` collections).

**Main thread**
- **The three draw lanes (xmodel / fx / static) rebuild as one multi-threaded island.** They read separate inputs and write separate lanes, so they now run in parallel instead of back to back. The island runner carries the union of the three systems' ordering edges.
- **Scene entity cells are kept across frames.** Most scene entities are placed script models that never move, and re-walking the BSP for each one every frame was most of the linking cost. There is a new allocation-free `dpvs_iw4::scene_ent_cells` walk, and the dyn-ent brush pass is skipped when nothing is admitted.
- **Bullet traces copy out only the entity collision the ray can reach.** This uses the same bounds test the trace already applied per geom.
- **Impact marks skip glass panes they cannot reach** before any material lookup. Material eligibility is cached per glass def per call.
- **HUD:** the map zone is no longer re-adopted (a disk stat) every frame. The 96 KiB render command buffer is reused instead of allocated and zeroed per element.
- **Menu expression dvars are indexed once** instead of re-tokenised on every lookup.
- **FX collision jobs go to workers only in batches of 16 or more.** Smaller batches ran slower than evaluating inline.
- The smodel bucket and skinned pack census lines log only when the counts change.

**Allocator**
- **The counting allocator is backed by mimalloc on Windows.** The process heap takes a lock on every call, and at the start of this work the heap alone was about a quarter of the main thread. Other platforms still use the system allocator, and the counting wrapper is unchanged.

**Bench**
- On Windows the bench report is written from `PostUpdate` of the frame that sends `AppExit`. `std::process::exit` there runs no `atexit` hook, so the report was never written.

## Testing
- Compared bench screenshots between master and this branch on the same spawn. The differences are within the run-to-run noise from foliage and viewmodel sway; I found no missing or wrong geometry, glass, lighting or marks. Worth a look on your side too, especially glass and model lighting on moving models.
- `cargo xtask publish-check` is clean. `cargo fmt --all` passes, and `cargo clippy --workspace --all-targets` adds no warnings on changed lines (the workspace already had some).
- Not run: `approved_tests`, which doesn't build on Windows (`std::os::unix`).
- Not measured: the heavy bot scenario for this final build. Earlier runs of the same changes with bots went from about 95 to about 180–200 fps.
