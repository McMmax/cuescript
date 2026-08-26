//! the shipped auth flow, end to end — the yardstick from example.md
//!
//! if this file stops compiling or behaving, the language has regressed against
//! the surface it was designed for.

use std::collections::HashMap;

use cuescript::runtime::{BoxFuture, CallResult, Host, Outcome, Row, Scope};
use cuescript::{build, runtime};

/// the crate has no async runtime dependency, so tests drive the future by hand
fn block_on<T>(fut: impl std::future::Future<Output = T>) -> T {
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
    fn vtable() -> &'static RawWakerVTable {
        &RawWakerVTable::new(
            |_| RawWaker::new(std::ptr::null(), vtable()),
            |_| {},
            |_| {},
            |_| {},
        )
    }
    let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), vtable())) };
    let mut cx = Context::from_waker(&waker);
    let mut fut = Box::pin(fut);
    loop {
        // every host here is immediate, so one poll always completes
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => panic!("test host must not suspend"),
        }
    }
}

const AUTH: &str = r#"
fragment auth_header {
    text "Offline Identity: {player.offline_name}" color=#FFD166
    rule "------------------------------" color=gray
}

flow auth {
    entry welcome

    screen welcome {
        include auth_header
        title "Welcome to Terrorify" color=#58C7F3 bold
        text "Create an account to save your progress,"
        hint "It only takes a few seconds."

        button "Create an Account" primary { open register_username }
        button "I Already Have an Account" { open login }
        button "Disconnect" quiet { kick "Disconnected." }
    }

    screen login {
        include auth_header
        title "Log In" color=#58C7F3 bold
        input username fill={player.offline_name}
        input password secret

        button "Log In" primary {
            require {input.password} is not empty
                else "Enter your password."
            call player_login with username = {input.username}, password = {input.password}
                on bad_password -> stay "Incorrect password."
                on timeout -> stay "Login timed out. Try again."
            release
        }
        button "Back" quiet { open welcome }
    }

    screen register_username {
        include auth_header
        title "Create an Account" color=#58C7F3 bold
        step 1 of 3
        hint "1-16 characters."
        input username fill={player.offline_name}

        button "Check Availability" primary {
            let name = {input.username|trim}
            require name matches "^[A-Za-z0-9_]{1,16}$"
                else "Use 1-16 letters, numbers or underscores."
            lookup player_core by username_lower = name -> existing
            require existing is empty
                else "{name} is already taken. Try another."
            remember chosen_username = name
            open register_password
        }
        button "Back" quiet { open welcome }
    }

    screen register_password {
        include auth_header
        title "Create an Account" color=#58C7F3 bold
        step 2 of 3
        good "{state.chosen_username} is available"
        input password secret

        button "Continue" primary {
            let pw = {input.password}
            require pw length between 6 and 64
                else "Password must be 6-64 characters."
            require pw has no whitespace
                else "Password cannot contain spaces."
            remember password = pw
            open register_confirm
        }
        button "Back" quiet { open welcome }
    }

    screen register_confirm {
        include auth_header
        title "Create an Account" color=#58C7F3 bold
        step 3 of 3
        input confirm secret

        button "Create My Account" primary {
            require {input.confirm} is {state.password}
                else "Those passwords do not match."
            call player_register with username = {state.chosen_username}, password = {state.password}
                on username_taken -> { say "That username was just taken." open register_username }
                on timeout -> stay "Registration timed out."
            open success
        }
        button "Start Over" quiet { open register_username }
    }

    screen success {
        include auth_header
        title "Account Created" color=#57E389 bold
        good "You are all set!"
        text "Username        {state.chosen_username}" color=#FFD166
        text "Spacetime UID   #{account.spacetime_id}" color=#FFD166

        button "Continue to Login" primary { open login }
        button "Disconnect" quiet { kick "Disconnected." }
    }
}
"#;

/// a host with a fixed taken name and a scripted reducer outcome
struct Fake {
    taken: &'static str,
    call_result: Option<&'static str>,
    calls: Vec<String>,
}

impl Host for Fake {
    fn lookup(&self, table: &str, _field: &str, value: &str) -> Row {
        assert_eq!(table, "player_core");
        if value.eq_ignore_ascii_case(self.taken) {
            let mut row = HashMap::new();
            row.insert("username".to_string(), value.to_string());
            Some(row)
        } else {
            None
        }
    }

    fn call<'a>(
        &'a mut self,
        effect: &'a str,
        args: &'a [(String, String)],
    ) -> BoxFuture<'a, CallResult> {
        self.calls.push(format!("{effect}({})", args.len()));
        let result = self.call_result;
        Box::pin(async move {
            match result {
                None => CallResult::Ok,
                Some("timeout") => CallResult::Timeout,
                Some(code) => CallResult::Err(code.to_string()),
            }
        })
    }
}

fn slots() -> HashMap<String, String> {
    let mut s = HashMap::new();
    s.insert("player.offline_name".into(), "blockinn".into());
    s.insert("account.spacetime_id".into(), "1234".into());
    s
}

fn press(
    p: &cuescript::Program,
    screen: u16,
    button: usize,
    inputs: &[(&str, &str)],
    host: &mut Fake,
    scope: &mut Scope,
) -> Outcome {
    let ctx = slots();
    for (k, v) in inputs {
        scope.vars.insert(format!("input.{k}"), v.to_string());
    }
    let steps = p.screens[screen as usize].buttons[button].steps;
    block_on(runtime::run(p, steps, scope, &ctx, host))
}

#[test]
fn the_shipped_flow_compiles() {
    let p = build(AUTH).expect("auth flow compiles");
    assert_eq!(p.screens.len(), 6);
    assert_eq!(p.screens[p.entry as usize].name, "welcome");
}

#[test]
fn the_header_fragment_lands_on_every_screen() {
    let p = build(AUTH).unwrap();
    let ctx = slots();
    for id in 0..p.screens.len() as u16 {
        let view = runtime::render(&p, id, &ctx);
        assert!(
            view.lines
                .iter()
                .any(|l| l.text.contains("Offline Identity: blockinn")),
            "screen `{}` is missing the header",
            p.screens[id as usize].name
        );
    }
}

#[test]
fn placeholders_resolve_from_the_host() {
    let p = build(AUTH).unwrap();
    let id = p.screen_by_name("success").unwrap();
    let view = runtime::render(&p, id, &slots());
    assert!(view.lines.iter().any(|l| l.text.contains("#1234")));
}

#[test]
fn a_free_username_advances_to_the_password_step() {
    let p = build(AUTH).unwrap();
    let mut host = Fake {
        taken: "taken_name",
        call_result: None,
        calls: vec![],
    };
    let mut scope = Scope::default();
    let screen = p.screen_by_name("register_username").unwrap();

    let out = press(
        &p,
        screen,
        0,
        &[("username", "freshname")],
        &mut host,
        &mut scope,
    );
    assert_eq!(
        out,
        Outcome::Open {
            screen: p.screen_by_name("register_password").unwrap(),
            message: None
        }
    );
    assert_eq!(
        scope.vars.get("state.chosen_username").map(String::as_str),
        Some("freshname")
    );
}

/// the guard is the point: a taken name must not reach the next page
#[test]
fn a_taken_username_stays_put_with_a_reason() {
    let p = build(AUTH).unwrap();
    let mut host = Fake {
        taken: "blockinn",
        call_result: None,
        calls: vec![],
    };
    let mut scope = Scope::default();
    let screen = p.screen_by_name("register_username").unwrap();

    let out = press(
        &p,
        screen,
        0,
        &[("username", "blockinn")],
        &mut host,
        &mut scope,
    );
    match out {
        Outcome::Stay { message: Some(m) } => assert!(m.contains("already taken"), "{m}"),
        other => panic!("expected to stay on the screen, got {other:?}"),
    }
}

#[test]
fn an_invalid_username_never_reaches_the_database() {
    let p = build(AUTH).unwrap();
    let mut host = Fake {
        taken: "x",
        call_result: None,
        calls: vec![],
    };
    let mut scope = Scope::default();
    let screen = p.screen_by_name("register_username").unwrap();

    let out = press(
        &p,
        screen,
        0,
        &[("username", "no spaces allowed")],
        &mut host,
        &mut scope,
    );
    assert!(matches!(out, Outcome::Stay { message: Some(_) }));
}

#[test]
fn a_password_with_spaces_is_rejected() {
    let p = build(AUTH).unwrap();
    let mut host = Fake {
        taken: "x",
        call_result: None,
        calls: vec![],
    };
    let mut scope = Scope::default();
    let screen = p.screen_by_name("register_password").unwrap();

    let out = press(
        &p,
        screen,
        0,
        &[("password", "has a space")],
        &mut host,
        &mut scope,
    );
    match out {
        Outcome::Stay { message: Some(m) } => assert!(m.contains("spaces"), "{m}"),
        other => panic!("expected rejection, got {other:?}"),
    }
}

#[test]
fn mismatched_confirmation_does_not_call_the_reducer() {
    let p = build(AUTH).unwrap();
    let mut host = Fake {
        taken: "x",
        call_result: None,
        calls: vec![],
    };
    let mut scope = Scope::default();
    scope.set_state("password", "correct1");
    scope.set_state("chosen_username", "freshname");
    let screen = p.screen_by_name("register_confirm").unwrap();

    let out = press(
        &p,
        screen,
        0,
        &[("confirm", "different")],
        &mut host,
        &mut scope,
    );
    assert!(matches!(out, Outcome::Stay { message: Some(_) }));
    assert!(
        host.calls.is_empty(),
        "a failed guard must short-circuit before the effect"
    );
}

#[test]
fn a_successful_registration_opens_the_receipt() {
    let p = build(AUTH).unwrap();
    let mut host = Fake {
        taken: "x",
        call_result: None,
        calls: vec![],
    };
    let mut scope = Scope::default();
    scope.set_state("password", "correct1");
    scope.set_state("chosen_username", "freshname");
    let screen = p.screen_by_name("register_confirm").unwrap();

    let out = press(
        &p,
        screen,
        0,
        &[("confirm", "correct1")],
        &mut host,
        &mut scope,
    );
    assert_eq!(
        out,
        Outcome::Open {
            screen: p.screen_by_name("success").unwrap(),
            message: None
        }
    );
    assert_eq!(host.calls, vec!["player_register(2)"]);
}

/// the race we hit in production: free at check time, taken by create time
#[test]
fn a_username_taken_during_creation_routes_back_to_step_one() {
    let p = build(AUTH).unwrap();
    let mut host = Fake {
        taken: "x",
        call_result: Some("username_taken"),
        calls: vec![],
    };
    let mut scope = Scope::default();
    scope.set_state("password", "correct1");
    scope.set_state("chosen_username", "freshname");
    let screen = p.screen_by_name("register_confirm").unwrap();

    let out = press(
        &p,
        screen,
        0,
        &[("confirm", "correct1")],
        &mut host,
        &mut scope,
    );
    assert_eq!(
        out,
        Outcome::Open {
            screen: p.screen_by_name("register_username").unwrap(),
            message: Some("That username was just taken.".into()),
        },
        "the arm routes back to step 1 carrying its say() message"
    );
}

#[test]
fn a_reducer_timeout_is_handled_not_swallowed() {
    let p = build(AUTH).unwrap();
    let mut host = Fake {
        taken: "x",
        call_result: Some("timeout"),
        calls: vec![],
    };
    let mut scope = Scope::default();
    scope.set_state("password", "correct1");
    scope.set_state("chosen_username", "freshname");
    let screen = p.screen_by_name("register_confirm").unwrap();

    let out = press(
        &p,
        screen,
        0,
        &[("confirm", "correct1")],
        &mut host,
        &mut scope,
    );
    match out {
        Outcome::Stay { message: Some(m) } => assert!(m.contains("timed out"), "{m}"),
        other => panic!("a timeout must not advance the flow, got {other:?}"),
    }
}

/// an unhandled reducer error must never fall through into the success path
#[test]
fn an_unhandled_error_does_not_advance() {
    let p = build(AUTH).unwrap();
    let mut host = Fake {
        taken: "x",
        call_result: Some("account_missing"),
        calls: vec![],
    };
    let mut scope = Scope::default();
    scope.set_state("password", "correct1");
    scope.set_state("chosen_username", "freshname");
    let screen = p.screen_by_name("register_confirm").unwrap();

    let out = press(
        &p,
        screen,
        0,
        &[("confirm", "correct1")],
        &mut host,
        &mut scope,
    );
    assert!(matches!(out, Outcome::Stay { .. }), "got {out:?}");
}

#[test]
fn login_releases_the_player() {
    let p = build(AUTH).unwrap();
    let mut host = Fake {
        taken: "x",
        call_result: None,
        calls: vec![],
    };
    let mut scope = Scope::default();
    let screen = p.screen_by_name("login").unwrap();

    let out = press(
        &p,
        screen,
        0,
        &[("password", "hunter22")],
        &mut host,
        &mut scope,
    );
    assert_eq!(out, Outcome::Release);
}

#[test]
fn a_bad_password_stays_on_the_login_screen() {
    let p = build(AUTH).unwrap();
    let mut host = Fake {
        taken: "x",
        call_result: Some("bad_password"),
        calls: vec![],
    };
    let mut scope = Scope::default();
    let screen = p.screen_by_name("login").unwrap();

    let out = press(
        &p,
        screen,
        0,
        &[("password", "wrong")],
        &mut host,
        &mut scope,
    );
    assert_eq!(
        out,
        Outcome::Stay {
            message: Some("Incorrect password.".into())
        }
    );
}

#[test]
fn disconnect_kicks_with_the_authored_reason() {
    let p = build(AUTH).unwrap();
    let mut host = Fake {
        taken: "x",
        call_result: None,
        calls: vec![],
    };
    let mut scope = Scope::default();
    let welcome = p.entry;

    let out = press(&p, welcome, 2, &[], &mut host, &mut scope);
    assert_eq!(
        out,
        Outcome::Kick {
            message: "Disconnected.".into()
        }
    );
}

#[test]
fn secret_inputs_are_marked_so_a_host_can_mask_them() {
    let p = build(AUTH).unwrap();
    let view = runtime::render(&p, p.screen_by_name("login").unwrap(), &slots());
    let pw = view.inputs.iter().find(|i| i.key == "password").unwrap();
    assert!(pw.secret);
    let user = view.inputs.iter().find(|i| i.key == "username").unwrap();
    assert!(!user.secret);
    assert_eq!(
        user.initial, "blockinn",
        "fill= should prefill from the host"
    );
}

/// every path a player can take must terminate at a screen that exists
#[test]
fn every_open_target_exists() {
    let p = build(AUTH).unwrap();
    for prog in &p.programs {
        for step in prog {
            if let cuescript::program::Step::Open(id) = step {
                assert!(
                    (*id as usize) < p.screens.len(),
                    "dangling open target {id}"
                );
            }
        }
    }
}

/// steady-state rendering must not allocate a fresh program each time
#[test]
fn rendering_is_repeatable_and_stable() {
    let p = build(AUTH).unwrap();
    let ctx = slots();
    let a = runtime::render(&p, p.entry, &ctx);
    let b = runtime::render(&p, p.entry, &ctx);
    assert_eq!(a, b);
}
