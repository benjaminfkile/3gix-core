# Laws

Every law function in `gx-core`, its units, and its determinism contract. The laws produce numbers for the renderer; they never produce hub bytes, but they are held to the determinism rule of `determinism.md` so a renderer and a conformance check agree bit for bit. No law knows what matter is: each reads only positions, masses, temperatures, and the optical channels of `matter-format.md` section 3.3.

## Bands

Format version 1 fixes three wavelength bands, `gx_core::radiance::BAND_EDGES = [700e-9, 600e-9, 500e-9, 400e-9]` meters, long to short wavelength:

| Band | From | To |
|---|---|---|
| 0 | 600 nm | 700 nm |
| 1 | 500 nm | 600 nm |
| 2 | 400 nm | 500 nm |

The three albedo values of a sample map to these bands in this order. Every per-band value below (`[f64; 3]`) is band 0 first.

## Constants

| Constant | Value | Unit | Source |
|---|---|---|---|
| `gravity::G` | 6.67430e-11 | m^3 kg^-1 s^-2 | CODATA 2018 |
| `gravity::C` | 299792458 | m/s | exact |
| `radiance::PLANCK` | 6.62607015e-34 | J s | CODATA 2018, exact |
| `radiance::SPEED_OF_LIGHT` | 299792458 | m/s | CODATA 2018, exact |
| `radiance::BOLTZMANN` | 1.380649e-23 | J/K | CODATA 2018, exact |
| `radiance::STEFAN_BOLTZMANN` | 5.670374419e-8 | W m^-2 K^-4 | CODATA 2018 |

## Functions

| Function | Inputs | Output | Determinism |
|---|---|---|---|
| `gravity::acceleration_at` | point (m), point masses (m, kg) | m/s^2 | sum in slice order |
| `gravity::potential_at` | point (m), point masses (m, kg) | J/kg | sum in slice order |
| `gravity::acceleration_from_section` | point (m), section | m/s^2 | sum in sample index order |
| `gravity::newtonian_accelerations` | bodies (m, m/s, kg) | m/s^2 per body | sums by ascending `frame_id` |
| `gravity::relativistic_corrections` | bodies | m/s^2 per body | first post-Newtonian terms only, sums by ascending `frame_id` |
| `gravity::oblateness_accelerations` | bodies with `j2`, radius (m), axis | m/s^2 per body | J2 force and reaction, sums by ascending `frame_id` |
| `gravity::accelerations` | bodies | m/s^2 per body | Newtonian plus relativistic plus oblateness |
| `rotation::torque` | moments (kg m^2), orientation, position (m), point masses | N m in frame axes | MacCullagh's formula, sum in slice order |
| `rotation::free_rotation` | orientation, angular momentum (kg m^2/s), moments, time (s) | orientation, angular momentum | splitting into fixed-axis rotations, exact for a symmetric frame |
| `integrate::step` | frame system, step (s), scheme | frame states | see `integrate` module docs |
| `integrate::advance` | frame system, target time (s) | frame states | fixed step count, see `determinism.md` |
| `history::History::state_at` | time (s) | frame system | nearest checkpoint at or before, then `advance` |
| `history::History::peek` | time (s) | frame system | as `state_at`, from the checkpoints already held, through a shared reference |
| `light::retarded_time` | point (m, root), time (s), history, frame id | retarded time (s), clamped flag, passes | fixed-point iteration from `t`, fixed stopping rule |
| `light::retarded_state` | point (m, root), time (s), history, frame id | root state at the retarded time | as `retarded_time`, then `History::peek` |
| `light::retarded_emitter` | lit point (m, root), time (s), history, emitter id | root position (m) | as `retarded_time` |
| `gravity::newtonian_field_at` | point (m), bodies | m/s^2 | point masses plus J2, sum by ascending `frame_id` |
| `observer::local_gravity` | offset from a frame (m, root axes), frame, system | m/s^2 | field minus the frame origin's acceleration |
| `observer::up_direction` | offset from a frame (m, root axes), frame, system | unit vector | against `local_gravity` |
| `observer::step_observer` | observer state, frame, system, probe, walking force (N), step (s), constants | observer state | `ceil(dt / (1/240 s))` equal substeps, fixed bisection rules |
| `observer::simulate` | as `step_observer`, step count | observer state, frame system | `step_observer` then one `advance` step per step |
| `radiance::spectral_radiance` | wavelength (m), temperature (K) | W m^-2 sr^-1 m^-1 | written-out exponential |
| `radiance::band_radiance` | temperature (K) | W m^-2 sr^-1 per band | 64-point midpoint rule per band, ascending wavelength |
| `radiance::total_radiance` | temperature (K) | W m^-2 sr^-1 | closed form `sigma T^4 / pi` |
| `radiance::emitted_band_radiance` | temperature (K), albedo per band | W m^-2 sr^-1 per band | `band_radiance * (1 - albedo)` |
| `emission::summarize` | section, minimum temperature (K) | position (m), power per band (W), radius (m) | sums in sample index order |
| `extinction::extinction_coefficient` | density (kg/m^3), attenuation (m^2/kg) | 1/m | one product |
| `extinction::transmittance` | density, attenuation, path (m) | ratio | written-out exponential |
| `extinction::optical_depth_along` | section, segment ends (m), steps | dimensionless | caller-fixed steps, summed in order |
| `lod::select_cells` | frame, camera (m), selection params | cell keys | fixed refinement order, sorted output |

### Gravity and rotation

`integrate::Scheme::Relativistic` is the default. It integrates the Newtonian point-mass part with the fourth order Yoshida scheme and adds the first post-Newtonian (Einstein-Infeld-Hoffmann, `beta = gamma = 1`) corrections, the J2 force of every oblate frame, and the torques on every frame with principal moments as half-step perturbation kicks around it, the velocity-dependent kick solved by the implicit midpoint rule. The global error is `O(h^4) + O(eps h^2)` with `eps` the size of the perturbation relative to Newtonian gravity. `Scheme::Oblate` drops the relativistic terms, and `Scheme::Yoshida4` and `Scheme::VelocityVerlet` are point-mass Newtonian only. The equations and their sources are in the `gravity`, `rotation`, and `integrate` module docs.

A frame's `angular_velocity` is relative to inertial axes, in the frame's own axes. The integrator advances every frame's orientation relative to root axes and stores it relative to its parent's new axes, so a frame never inherits its parent's spin.

### Light time

`light` solves `t_r = t - |x_obs - x_frame(t_r)| / c` against a `History` by fixed-point iteration from `t_r = t`. The map contracts by `v / c < 1` for any observer position, so it converges from any start, typically in two to four passes. It stops when two iterates differ by at most 1e-9 s, or by two units in the last place of `t` where an `f64` time cannot resolve 1e-9 s, or after 32 passes. A retarded time before the history's epoch is clamped to the epoch and reported as clamped. Only the receiving point's position at `t` enters, so the solution is well defined at any speed of the observer. Drawing uses `retarded_state`; lighting uses `retarded_emitter` with the lit point as the receiver. Extend the history to `t` first so each pass integrates at most one checkpoint interval.

### Observer

`observer` is the observer's body in walk mode, a point at its lowest point with the constants of `ObserverParams` (defaults: 80 kg, 1.75 m, 0.7 m^2, drag coefficient 1.0, 985 kg/m^3, friction 0.8). Its state is relative to one frame, in root axes. In each substep of at most 1/240 s: half a step of quadratic drag `0.5 rho |v_rel|^2 Cd A` for fluid, gas, and plasma samples, applied with its exact solution so it never reverses the motion; velocity Verlet under gravity (Newtonian point masses plus J2, minus the frame origin's acceleration) and buoyancy `-rho_local V g` with `V = mass / density`; contact; and another half step of drag. Contact with a solid sample moves the observer out along the surface normal (the density gradient, refined by fitting a plane to the solid boundary found by bisection), removes any velocity into the solid (the normal impulse), applies the walking force tangent to the surface, limited by `friction * normal force`, and lets friction, limited by what is left of that, drive the tangential velocity relative to the matter to zero except along the walking force. The renderer supplies the matter through `MatterProbe`, with each sample's velocity (the frame's rotation at that point).

### Radiance

`spectral_radiance` is Planck's law, `2 h c^2 / lambda^5 / (exp(h c / (lambda k_B T)) - 1)`. It returns 0 for a temperature or wavelength at or below 0 and when the exponent exceeds 700, so cold matter at short wavelength never overflows.

`band_radiance` integrates it over each band with a 64-point midpoint rule in `f64`: `width = (hi - lo) / 64`, then the sum over `j` from 0 to 63 of `spectral_radiance(lo + (j + 0.5) * width) * width`. **The rule and the point count are part of the contract.** Changing either changes every band value.

`emitted_band_radiance` applies Kirchhoff's law for opaque matter: emissivity is `1 - albedo` per band.

### Emission

`summarize` reduces every non-vacuum sample at or above a minimum temperature to one emitter: the power-weighted centroid of the sample centers, the power per band in watts, and the unweighted root mean square distance of the sample centers from the centroid. A sample's power in a band is `emitted_band_radiance * pi * 6 * s^2` for sub-cube edge `s`: hemispherical radiance over the six faces of the sub-cube. This is a deliberate v1 approximation that ignores faces hidden by hot neighbours. Returns `None` when no sample qualifies.

### Extinction

`extinction_coefficient` is `density * attenuation`, per meter. `transmittance` is `exp(-k * path)`. `optical_depth_along` samples the midpoints of `steps` equal parts of a segment, looks up the sample whose sub-cube holds each midpoint, and sums `k * part length`; midpoints outside the cell add nothing.

### Cell selection

`select_cells` walks a frame's octree from depth 0. A cell's projected edge in pixels is `edge / max(distance, edge) * view_height_px / (2 tan(fov / 2))`, with `distance` from the camera to the nearest point of the cell. A cell is refined into its eight children while that exceeds `pixel_error` and its depth is below the frame's `max_depth`. Cells farther than `4 * root_extent` are culled. Refinement runs largest projected edge first (ties by ascending `(depth, x, y, z)`) and stops as soon as the next refinement would exceed `max_cells`, so the result never holds more than `max_cells` cells. The output is sorted by `(depth, x, y, z)`.

## How the determinism is kept

- Only `+`, `-`, `*`, `/`, `sqrt`, `round`, and `floor`-like truncation, all exact or correctly rounded in IEEE 754.
- `exp`, `expm1`, `sin`, `cos`, and `tan` are written out in a private module (`detmath`): range reduction by an exactly split `ln 2`, a fixed-length Taylor series in Horner form, and scaling by an exact power of two. The platform math library is never called by a law.
- Every sum runs in a fixed order with plain operators: no fused multiply add, no SIMD reductions, no hash map iteration.
