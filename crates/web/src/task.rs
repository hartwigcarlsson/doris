//! Tasks that belong to the page they were started on.
//!
//! A page asks the server for something and goes on when the answer comes.
//! By then the user may have gone to another page: the page's signals are
//! disposed, and reading one panics, which aborts the release wasm. So a
//! page's task is dropped when the path changes: the request has been
//! sent and the server finishes it, but nothing more of the page runs.
//!
//! Pages use [`spawn_local`] from here. What outlives a page (the session,
//! the company list, the header) uses `leptos::task::spawn_local`.

use leptos::prelude::*;
use leptos_router::hooks::use_location;
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

thread_local! {
    /// The running page tasks: the path each was started on, and its flag.
    static TASKS: RefCell<Vec<(String, Rc<Cell<bool>>)>> = const { RefCell::new(Vec::new()) };
}

/// `task`, until `left` is set: then it finishes without running further.
struct UntilLeft<F> {
    task: Pin<Box<F>>,
    left: Rc<Cell<bool>>,
}

impl<F: Future<Output = ()>> Future for UntilLeft<F> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.left.get() {
            return Poll::Ready(());
        }
        self.task.as_mut().poll(cx)
    }
}

/// Marks every task started on another path than `path` as left, and
/// forgets those that have finished.
fn leave(tasks: &mut Vec<(String, Rc<Cell<bool>>)>, path: &str) {
    tasks.retain(|(started_on, left)| {
        // The task holds the other reference while it runs.
        let running = Rc::strong_count(left) > 1;
        if running && started_on != path {
            left.set(true);
        }
        running && started_on == path
    });
}

/// Starts `task` for the page at the current path. It is dropped, at its
/// next `await`, once the user has left for another path.
pub fn spawn_local(task: impl Future<Output = ()> + 'static) {
    let left = Rc::new(Cell::new(false));
    let path = window().location().pathname().unwrap_or_default();
    TASKS.with_borrow_mut(|tasks| {
        leave_finished(tasks);
        tasks.push((path, left.clone()));
    });
    leptos::task::spawn_local(UntilLeft {
        task: Box::pin(task),
        left,
    });
}

/// Forgets the tasks that have finished.
fn leave_finished(tasks: &mut Vec<(String, Rc<Cell<bool>>)>) {
    tasks.retain(|(_, left)| Rc::strong_count(left) > 1);
}

/// Inside the router: drops the tasks of the page that was left whenever
/// the path changes. A change of the query alone (the chosen year) is the
/// same page.
#[component]
pub fn PageTasks() -> impl IntoView {
    let path = use_location().pathname;
    Effect::new(move |_| {
        let path = path.get();
        TASKS.with_borrow_mut(|tasks| leave(tasks, &path));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::task::Waker;

    /// A task that is pending until `done` is set, and counts its polls.
    fn task(done: Rc<Cell<bool>>, polls: Rc<Cell<u32>>) -> impl Future<Output = ()> {
        std::future::poll_fn(move |_| {
            polls.set(polls.get() + 1);
            if done.get() {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
    }

    #[test]
    fn a_task_that_was_left_runs_no_further() {
        let (done, polls, left) = (Rc::default(), Rc::default(), Rc::new(Cell::new(false)));
        let mut guarded = Box::pin(UntilLeft {
            task: Box::pin(task(Rc::clone(&done), Rc::clone(&polls))),
            left: left.clone(),
        });
        let mut cx = Context::from_waker(Waker::noop());
        assert_eq!(guarded.as_mut().poll(&mut cx), Poll::Pending);
        assert_eq!(polls.get(), 1);
        // The user leaves; then the answer comes.
        left.set(true);
        done.set(true);
        assert_eq!(guarded.as_mut().poll(&mut cx), Poll::Ready(()));
        assert_eq!(polls.get(), 1, "the page's code did not run again");
    }

    #[test]
    fn a_task_that_was_not_left_runs_to_its_end() {
        let (done, polls) = (Rc::default(), Rc::default());
        let mut guarded = Box::pin(UntilLeft {
            task: Box::pin(task(Rc::clone(&done), Rc::clone(&polls))),
            left: Rc::new(Cell::new(false)),
        });
        let mut cx = Context::from_waker(Waker::noop());
        assert_eq!(guarded.as_mut().poll(&mut cx), Poll::Pending);
        done.set(true);
        assert_eq!(guarded.as_mut().poll(&mut cx), Poll::Ready(()));
        assert_eq!(polls.get(), 2);
    }

    #[test]
    fn leaving_marks_the_tasks_of_other_paths_and_forgets_finished_ones() {
        let flag = || Rc::new(Cell::new(false));
        // Each running task holds a second reference to its flag.
        let (here, there, finished) = (flag(), flag(), flag());
        let _running = (here.clone(), there.clone());
        let mut tasks = vec![
            ("/vouchers".to_owned(), here.clone()),
            ("/accounts".to_owned(), there.clone()),
            ("/accounts".to_owned(), finished.clone()),
        ];
        drop(finished);

        leave(&mut tasks, "/vouchers");

        assert!(!here.get(), "the page that is open keeps its task");
        assert!(there.get(), "the page that was left loses its task");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].0, "/vouchers");
    }
}
