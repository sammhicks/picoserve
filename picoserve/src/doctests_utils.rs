use crate::routing::{PathRouter, Router};

pub fn router(_: Router<impl PathRouter>) {}

pub fn router_with_state<State>(_: Router<impl PathRouter<State>, State>) {}
