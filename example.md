# CueScript by example

The cracked auth flow Internode ships today, written as CueScript. This is the
same flow currently hardcoded in `internode-core/src/limbo/auth_dialog.rs` and
`auth_handler.rs` — roughly 700 lines of Rust across two files — so it doubles
as the yardstick for whether the language is worth having.

---

## The whole flow

```
# fragments are included by name, so the chrome is authored once and can be
# omitted on any screen that does not want it
fragment auth_header {
    text  "Offline Identity: {player.offline_name}" color=#FFD166
    rule  "─" color=gray
}

flow auth {
    entry welcome

    # ── Welcome ──────────────────────────────────────────────────────────
    screen welcome {
        include auth_header
        title "Welcome to Terrorify" color=#58C7F3 bold

        text "Create an account to save your progress,"
        text "or log in if you already have one."
        hint "It only takes a few seconds."

        button "Create an Account" primary   { open register.username }
        button "I Already Have an Account"   { open login }
        button "Disconnect" quiet            { kick "Disconnected." }
    }

    # ── Login ────────────────────────────────────────────────────────────
    screen login {
        include auth_header
        title "Log In" color=#58C7F3 bold

        text "Welcome back!"
        hint "Enter your details to continue."

        input username fill={account.username | player.offline_name}
        input password secret

        button "Log In" primary {
            require {input.password} is not empty
                else "Enter your password."

            call player_login with username = {input.username},
                                   password = {input.password}
                on bad_password  -> stay "Incorrect password."
                on wrong_username -> stay "That username does not match this account."
                on ip_mismatch   -> stay "Unrecognised connection — enter your username too."
                on not_registered -> open register.username
                                     say "No account found. Create one to get started."
                on timeout       -> stay "Login timed out. Try again."

            release                     # internode's own word: let them out of limbo
        }

        button "Create an Account Instead" { open register.username }
        button "Back" quiet               { open welcome }
    }

    # ── Register 1/3 — username ──────────────────────────────────────────
    screen register.username {
        include auth_header
        title "Create an Account" color=#58C7F3 bold
        step  1 of 3

        text "Pick your username"
        hint "This is the name others will see on the network."
        hint "1-16 characters — letters, numbers and underscores."

        input username fill={player.offline_name}

        button "Check Availability" primary {
            let name = {input.username|trim}

            require name matches "^[A-Za-z0-9_]{1,16}$"
                else "Use 1-16 letters, numbers or underscores."

            # cache-backed read: a DashMap hit, not a round trip
            lookup player_core by username_lower = name -> existing
            require existing is empty
                else "{name} is already taken. Try another."

            remember chosen_username = name
            open register.password
        }

        button "Back" quiet { open welcome }
    }

    # ── Register 2/3 — password ──────────────────────────────────────────
    screen register.password {
        include auth_header
        title "Create an Account" color=#58C7F3 bold
        step  2 of 3

        good "{state.chosen_username} is available"
        text "Now choose a password"
        hint "Between {rule.password_min} and {rule.password_max} characters."
        hint "No spaces."

        input password secret

        button "Continue" primary {
            let pw = {input.password}

            require pw length between {rule.password_min} and {rule.password_max}
                else "Password must be {rule.password_min}-{rule.password_max} characters."
            require pw has no whitespace
                else "Password cannot contain spaces."

            remember password = pw
            open register.confirm
        }

        button "Back" quiet { open welcome }
    }

    # ── Register 3/3 — confirm ───────────────────────────────────────────
    screen register.confirm {
        include auth_header
        title "Create an Account" color=#58C7F3 bold
        step  3 of 3

        good "Password accepted"
        text "Confirm your password"
        hint "Type it once more so we know it was not a typo."
        text "Registering as {state.chosen_username}" color=#FFD166

        input confirm secret

        button "Create My Account" primary {
            require {input.confirm} is {state.password}
                else "Those passwords do not match. Try again."

            call player_register with username = {state.chosen_username},
                                      password = {state.password}
                # the advisory lookup above can go stale between check and
                # create — that race is expected, so it routes back to step 1
                on username_taken    -> open register.username
                                        say "That username was just taken."
                on invalid_username  -> open register.username
                                        say "Invalid username."
                on timeout           -> stay "Registration timed out. Try again."

            open success
        }

        button "Start Over" quiet { open register.username }
    }

    # ── Receipt ──────────────────────────────────────────────────────────
    screen success {
        include auth_header
        title "Account Created" color=#57E389 bold

        good "You're all set!"
        hint "Keep these details for your records:"
        rule "─" color=gray
        text "Username        {state.chosen_username}"      color=#FFD166
        text "Created         {now|date:long}"              color=#FFD166
        text "Spacetime UID   #{account.spacetime_id}"      color=#FFD166
        text "Linked Identity {player.offline_name}"        color=#FFD166
        hint "                {player.offline_uuid}"
        rule "─" color=gray
        text "Log in now to finish and start playing."

        button "Continue to Login" primary { open login }
        button "Disconnect" quiet          { kick "Disconnected." }
    }
}
```

---

## What the compiler does with it

Nothing above survives to the hot path as text:

- `"Offline Identity: "` and every other literal are interned into one blob;
  emitting them is a `memcpy`.
- `{player.offline_name}` becomes slot `3`. The name exists only in the editor.
- `matches "^[A-Za-z0-9_]{1,16}$"` compiles once, at publish.
- `include auth_header` is inlined, so a fragment costs nothing at render.
- `step 1 of 3` expands to a `text` node — sugar, not a runtime concept.
- The `require`/`else` pairs become `JumpIfFalse` with precomputed targets.

`lookup` is a synchronous cache read, safe on a tick. `call` is the only await
point and runs on a task, so no screen in this file can stall the tick thread.

## What it replaces

| Today | With CueScript |
|---|---|
| `auth_dialog.rs` — screen builders, palette, chrome | this file |
| `auth_handler.rs::on_custom_click` — the step logic | the `button` bodies |
| Rebuild + copy binary + restart pods to change a dash | edit, validate, publish |
| Copy-paste of the header across six screens | one `fragment`, six `include`s |
| Flow visible only by reading Rust | a graph the panel can draw |

## Notes for the reader

- `rule` is a component with a character, colour and width — which is how
  "make the divider editable" stops needing a code change.
- `say` writes to the surface the player is on. Inside a dialog that is the
  dialog's error line, not chat, because the player cannot see chat from a
  dialog. The surface profile decides.
- `release` and `kick` are Internode's own vocabulary; `transfer node:nae-cracked`
  is the same idea for the cracked hand-off.
- `{rule.password_min}` comes from the host manifest, so the copy cannot drift
  from the rule the server actually enforces — the exact drift we hit when the
  password screen advertised passphrases while the code rejected spaces.
