//! The subsurface side of the client script: plain `wl_surface`s, made into
//! subsurfaces of a window, a popup or each other, and taken apart again.
//! `subsurface_depth`'s tests drive it through [`Op::Sub`](super::Op::Sub).
//!
//! Every drawn subsurface attaches one of two buffers made once per client
//! ([`Buffers`]), not a buffer of its own: a tree thousands deep would
//! otherwise hit the per-client pool, buffer and fd caps (`shm_pools.rs`,
//! `wl_buffers.rs`, `client_fds.rs`) or the fd limit, and be refused for the
//! wrong reason.

use wayland_client::QueueHandle;
use wayland_client::protocol::{wl_buffer, wl_subsurface, wl_surface};

use super::{Globals, MARKED_BGRA, Made, STEP, TestClient, solid_buffer};

/// Each drawn subsurface's size: square, and small enough that a chain
/// [`CAP`](super::CAP) deep, climbing [`STEP`] right and down per level,
/// still ends on screen -- under a popup chain that deep too.
pub(in crate::compositor) const SUB_SIZE: i32 = 4;

/// Every drawn subsurface but a chain's last, as BGRA bytes.
const SUB_BGRA: [u8; 4] = [0xE0, 0xE0, 0x20, 0xFF];

/// A surface a subsurface op names: as a parent, or as the one it acts on.
#[derive(Clone, Copy, Debug)]
pub(in crate::compositor) enum Node {
    /// The `n`-th toplevel this client mapped.
    Window(usize),
    /// The `n`-th popup this client made.
    Popup(usize),
    /// The `n`-th plain surface this client made ([`SubOp`]).
    Surface(usize),
}

/// One subsurface request (or a few that belong together), sent without a
/// round trip like every other [`Op`](super::Op).
#[derive(Clone, Copy, Debug)]
pub(in crate::compositor) enum SubOp {
    /// A `wl_surface` with no role, given a [`SUB_SIZE`] buffer and
    /// committed -- a subsurface is drawn only below a parent with a buffer,
    /// so a tree built below this one shows once it is attached. It becomes
    /// the next surface index.
    Surface,
    /// `len` new surfaces, each a subsurface of the one before it, the
    /// first of `parent`: the next `len` surface indices, outermost first.
    /// Each is placed [`STEP`] right and down of its parent, set
    /// desynchronized unless `sync`, given a [`SUB_SIZE`] buffer if `draw`
    /// (the last in [`MARKED_BGRA`]), and committed.
    ///
    /// A synchronized chain shows once its root's parent commits
    /// ([`SubOp::Commit`]); a desynchronized one as it is built.
    Chain {
        parent: Node,
        len: usize,
        sync: bool,
        draw: bool,
    },
    /// `get_subsurface` for plain surface `surface` under `parent`, placed
    /// [`STEP`] right and down of it and desynchronized; shown once
    /// `surface` commits. Any earlier `wl_subsurface` it had is left as it
    /// was.
    Attach { surface: usize, parent: Node },
    /// `wl_subsurface.destroy` on plain surface `surface`'s newest
    /// `wl_subsurface`, keeping the surface and whatever hangs below it.
    Detach(usize),
    /// `wl_surface.destroy` on plain surface `surface`, which orphans the
    /// subsurfaces below it, each keeping its own `wl_subsurface`.
    DestroySurface(usize),
    /// `wl_surface.commit` on `node`.
    Commit(Node),
}

/// A plain surface the script made, and its newest `wl_subsurface`.
pub(super) struct Surface {
    surface: wl_surface::WlSurface,
    subsurface: Option<wl_subsurface::WlSubsurface>,
}

/// The buffers every drawn subsurface shares: `plain` in [`SUB_BGRA`],
/// `marked` in [`MARKED_BGRA`].
pub(super) struct Buffers {
    plain: wl_buffer::WlBuffer,
    marked: wl_buffer::WlBuffer,
}

/// The `wl_surface` behind `node`.
fn surface_of(made: &Made, node: Node) -> Result<wl_surface::WlSurface, String> {
    Ok(match node {
        Node::Window(i) => made.windows.get(i).ok_or("no such window")?.surface.clone(),
        Node::Popup(i) => made.popups.get(i).ok_or("no such popup")?.surface.clone(),
        Node::Surface(i) => made
            .surfaces
            .get(i)
            .ok_or("no such surface")?
            .surface
            .clone(),
    })
}

/// The shared buffers, made on first use.
fn buffers<'a>(
    made: &'a mut Made,
    globals: &Globals,
    qh: &QueueHandle<TestClient>,
) -> Result<&'a Buffers, String> {
    if made.sub_buffers.is_none() {
        let (plain, _, _) = solid_buffer(&globals.shm, qh, SUB_SIZE, SUB_SIZE, SUB_BGRA)?;
        let (marked, _, _) = solid_buffer(&globals.shm, qh, SUB_SIZE, SUB_SIZE, MARKED_BGRA)?;
        made.sub_buffers = Some(Buffers { plain, marked });
    }
    made.sub_buffers
        .as_ref()
        .ok_or_else(|| "no buffers".to_owned())
}

pub(super) fn run(
    op: SubOp,
    globals: &Globals,
    qh: &QueueHandle<TestClient>,
    made: &mut Made,
) -> Result<(), String> {
    match op {
        SubOp::Surface => {
            let surface = globals.compositor.create_surface(qh, ());
            surface.attach(Some(&buffers(made, globals, qh)?.plain), 0, 0);
            surface.damage(0, 0, SUB_SIZE, SUB_SIZE);
            surface.commit();
            made.surfaces.push(Surface {
                surface,
                subsurface: None,
            });
        }
        SubOp::Chain {
            parent,
            len,
            sync,
            draw,
        } => {
            let mut parent = surface_of(made, parent)?;
            for i in 0..len {
                let surface = globals.compositor.create_surface(qh, ());
                let subsurface = globals
                    .subcompositor
                    .get_subsurface(&surface, &parent, qh, ());
                subsurface.set_position(STEP, STEP);
                if !sync {
                    subsurface.set_desync();
                }
                if draw {
                    let buffers = buffers(made, globals, qh)?;
                    let buffer = if i + 1 == len {
                        &buffers.marked
                    } else {
                        &buffers.plain
                    };
                    surface.attach(Some(buffer), 0, 0);
                    surface.damage(0, 0, SUB_SIZE, SUB_SIZE);
                }
                surface.commit();
                made.surfaces.push(Surface {
                    surface: surface.clone(),
                    subsurface: Some(subsurface),
                });
                parent = surface;
            }
        }
        SubOp::Attach { surface, parent } => {
            let parent = surface_of(made, parent)?;
            let entry = made.surfaces.get_mut(surface).ok_or("no such surface")?;
            let subsurface = globals
                .subcompositor
                .get_subsurface(&entry.surface, &parent, qh, ());
            subsurface.set_position(STEP, STEP);
            subsurface.set_desync();
            entry.subsurface = Some(subsurface);
        }
        SubOp::Detach(surface) => {
            made.surfaces
                .get_mut(surface)
                .ok_or("no such surface")?
                .subsurface
                .take()
                .ok_or("that surface has no wl_subsurface")?
                .destroy();
        }
        SubOp::DestroySurface(surface) => {
            made.surfaces
                .get(surface)
                .ok_or("no such surface")?
                .surface
                .destroy();
        }
        SubOp::Commit(node) => surface_of(made, node)?.commit(),
    }
    Ok(())
}
