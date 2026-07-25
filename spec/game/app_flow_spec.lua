local actions = require("game.input.actions")
local App = require("game.app")
local fake_result = require("game.fake_result")
local hit = require("game.ui.hit")
local t = require("spec.support.runner")
local viewport = require("game.ui.viewport")

---@param app App
---@param id string
local function click_widget(app, id)
    local screen = assert(app.stack:current())
    ---@cast screen Menu
    local widget = assert(hit.find(screen.def.layout(screen.state), id), "missing widget " .. id)
    local x, y = viewport.to_actual(
        app.transform,
        widget.rect.x + widget.rect.w / 2,
        widget.rect.y + widget.rect.h / 2
    )
    app:event({ kind = "click", x = x, y = y, button = 1 })
end

---@param app App
local function reach_fake_match(app)
    click_widget(app, "play")
    click_widget(app, "next")
    click_widget(app, "formation_1-1-2")
    click_widget(app, "next")
    click_widget(app, "tactic_press_high")
    click_widget(app, "kickoff")
    t.eq(app:current_route(), "match")
end

t.describe("product application flow", function()
    t.it("drives title through deterministic result and repeated matches", function()
        local app = App.new({ actual_w = 1280, actual_h = 800 })
        reach_fake_match(app)
        t.eq(app.session.formation_id, "1-1-2")
        t.eq(app.session.tactic_id, "press_high")

        click_widget(app, "complete")
        t.eq(app:current_route(), "result")
        t.eq(app.session.match_number, 1)
        local first_seed = assert(app.session.last_result).seed

        click_widget(app, "rematch")
        t.eq(app:current_route(), "match")
        click_widget(app, "complete")
        t.eq(app:current_route(), "result")
        t.eq(app.session.match_number, 2)
        t.eq(assert(app.session.last_result).seed, 2)
        t.is_true(first_seed ~= app.session.last_result.seed)

        click_widget(app, "rematch")
        click_widget(app, "complete")
        t.eq(app:current_route(), "result")
        t.eq(app.session.match_number, 3)
        t.eq(assert(app.session.last_result).seed, 3)

        click_widget(app, "change_plan")
        t.eq(app:current_route(), "formation")
        t.eq(app.session.formation_id, "1-1-2")
        t.eq(app.session.tactic_id, "press_high")
        t.eq(#app.session.starter_ids, 5)
    end)

    t.it("supports every result exit without losing the intended choices", function()
        local app = App.new()
        reach_fake_match(app)
        click_widget(app, "complete")
        click_widget(app, "change_lineup")
        t.eq(app:current_route(), "squad")
        t.eq(#app.session.starter_ids, 5)

        app:handle_action({ go = "formation", starter_ids = app.session.starter_ids })
        app:handle_action({
            go = "tactic",
            formation_id = app.session.formation_id,
        })
        app:handle_action({ go = "match", tactic_id = app.session.tactic_id })
        click_widget(app, "complete")
        click_widget(app, "main_menu")
        t.eq(app:current_route(), "title")
    end)

    t.it("preserves formation and tactic choices while navigating backward", function()
        local app = App.new()
        click_widget(app, "play")
        click_widget(app, "next")
        click_widget(app, "formation_1-1-2")
        click_widget(app, "back")
        t.eq(app:current_route(), "squad")
        t.eq(app.session.formation_id, "1-1-2")

        click_widget(app, "next")
        click_widget(app, "next")
        click_widget(app, "tactic_counter")
        click_widget(app, "back")
        t.eq(app:current_route(), "formation")
        t.eq(app.session.tactic_id, "counter")
        click_widget(app, "next")
        local tactic_menu = assert(app.stack:current())
        ---@cast tactic_menu Menu
        t.eq(tactic_menu.state.selected, "counter")
    end)

    t.it("maps keyboard and gamepad through nested shell routes", function()
        local app = App.new()
        app:event({ kind = "key", key = "down" })
        app:event({ kind = "key", key = "down" })
        app:event({ kind = "gamepad", button = "a" })
        t.eq(app:current_route(), "help")
        app:event({ kind = "gamepad", button = "b" })
        t.eq(app:current_route(), "title")
    end)

    t.it("keeps the showcase combat-disabled and exposes a separate prototype path", function()
        local app = App.new()
        click_widget(app, "combat_prototype")
        t.eq(app:current_route(), "squad")
        t.eq(app.session.combat_enabled, true)
        t.eq(assert(require("game.session").build_request(app.session, 4)).combat_enabled, true)

        app:show_title()
        click_widget(app, "play")
        t.eq(app.session.combat_enabled, false)
        t.eq(assert(require("game.session").build_request(app.session, 5)).combat_enabled, false)
    end)

    t.it("backs out of credits and handles quit deliberately", function()
        local app = App.new()
        click_widget(app, "credits")
        t.eq(app:current_route(), "credits")
        app:event({ kind = "key", key = "escape" })
        t.eq(app:current_route(), "title")
        click_widget(app, "quit")
        t.eq(app.quit_requested, true)
    end)

    t.it("persists settings and resumes a paused fake fixture", function()
        local saved = nil
        local storage = {
            read = function()
                return nil
            end,
            write = function(contents)
                saved = contents
                return true
            end,
        }
        local app = App.new({ settings_storage = storage })
        click_widget(app, "settings")
        app:event(actions.event("left"))
        t.near(app.settings.master_volume, 0.9)
        click_widget(app, "fullscreen")
        t.eq(app.settings.fullscreen, true)
        click_widget(app, "back")
        t.eq(app:current_route(), "title")
        t.is_true(assert(saved):match("master_volume=0.90") ~= nil)

        reach_fake_match(app)
        app:event({ kind = "key", key = "p" })
        t.eq(app:current_route(), "pause")
        app:event({ kind = "gamepad", button = "start" })
        t.eq(app:current_route(), "match")
    end)

    t.it("requires confirmation before restarting a paused fixture", function()
        local app = App.new()
        reach_fake_match(app)
        app:event({ kind = "key", key = "p" })
        click_widget(app, "restart")
        t.eq(app:current_route(), "pause")
        local paused = assert(app.stack:current())
        ---@cast paused Menu
        t.eq(paused.state.confirm_restart, true)
        click_widget(app, "restart")
        t.eq(app:current_route(), "match")
        t.eq(app.session.match_number, 0)
    end)

    t.it("returns from nested pause routes and can leave for the title", function()
        local storage = {
            read = function()
                return nil
            end,
            write = function()
                return true
            end,
        }
        local app = App.new({ settings_storage = storage })
        reach_fake_match(app)
        app:event({ kind = "key", key = "p" })
        click_widget(app, "controls")
        t.eq(app:current_route(), "help")
        app:event({ kind = "gamepad", button = "b" })
        t.eq(app:current_route(), "pause")
        click_widget(app, "settings")
        t.eq(app:current_route(), "settings")
        click_widget(app, "back")
        t.eq(app:current_route(), "pause")
        click_widget(app, "main_menu")
        t.eq(app:current_route(), "title")
    end)
end)

t.describe("fake match adapter", function()
    t.it("produces identical results for identical requests", function()
        local app = App.new()
        local request = assert(require("game.session").build_request(app.session, 41))
        local first = fake_result.for_request(request)
        local second = fake_result.for_request(request)
        t.eq(first.home_score, second.home_score)
        t.eq(first.away_score, second.away_score)
        t.eq(first.mvp_player_id, second.mvp_player_id)
        t.eq(first.home_stats.possession, second.home_stats.possession)
    end)

    t.it("cancels back to tactical setup", function()
        local app = App.new()
        reach_fake_match(app)
        click_widget(app, "cancel")
        t.eq(app:current_route(), "tactic")
        t.eq(app.session.formation_id, "1-1-2")
        t.eq(app.session.tactic_id, "press_high")
    end)
end)
