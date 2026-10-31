//! A secret game of Snake on the calendar's year view (`:snake` in the
//! calendar). The snake moves from day to day the way the cursor does there,
//! along the week's rows and down the weekday's columns, crossing to the month
//! beside, above or below at a month's edge. The year's edges are walls, and
//! filling every day of the year wins.

use std::collections::hash_map::RandomState;
use std::collections::{HashSet, VecDeque};
use std::hash::BuildHasher;
use std::time::{Duration, Instant};

use chrono::{Datelike, NaiveDate};

use crate::calendar::{step_across, step_down};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

impl Dir {
    fn opposite(self) -> Dir {
        match self {
            Dir::Left => Dir::Right,
            Dir::Right => Dir::Left,
            Dir::Up => Dir::Down,
            Dir::Down => Dir::Up,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
    /// Waiting for the first direction key.
    Ready,
    Playing,
    /// Hit a wall or itself.
    Lost,
    /// Filled the year.
    Won,
}

/// What's on a day, for drawing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cell {
    Head,
    Body,
    Food,
    Empty,
}

pub struct Snake {
    pub year: i32,
    /// The days the snake is on, its head first.
    body: VecDeque<NaiveDate>,
    occupied: HashSet<NaiveDate>,
    heading: Dir,
    /// Turns typed but not made yet, so two quick keys both count.
    turns: VecDeque<Dir>,
    pub food: Option<NaiveDate>,
    /// How much food has been eaten.
    pub score: u32,
    pub state: State,
    rng: u64,
    /// When the snake next moves, once it's moving.
    next_step: Option<Instant>,
}

/// The most turns kept waiting at once.
const QUEUED_TURNS: usize = 2;

impl Snake {
    /// A snake three days long ending on `start`, facing right, with the
    /// food placed with `seed`.
    pub fn new(start: NaiveDate, seed: u64) -> Self {
        let mut body = VecDeque::from([start]);
        while body.len() < 3
            && let Some(before) = body.back().and_then(|&day| step_across(day, false))
            && before.year() == start.year()
        {
            body.push_back(before);
        }
        let mut snake = Snake {
            year: start.year(),
            occupied: body.iter().copied().collect(),
            body,
            heading: Dir::Right,
            turns: VecDeque::new(),
            food: None,
            score: 0,
            state: State::Ready,
            // xorshift can't start from zero.
            rng: seed | 1,
            next_step: None,
        };
        snake.place_food();
        snake
    }

    pub fn head(&self) -> NaiveDate {
        self.body[0]
    }

    pub fn cell(&self, day: NaiveDate) -> Cell {
        if day == self.head() {
            Cell::Head
        } else if self.occupied.contains(&day) {
            Cell::Body
        } else if self.food == Some(day) {
            Cell::Food
        } else {
            Cell::Empty
        }
    }

    pub fn over(&self) -> bool {
        matches!(self.state, State::Lost | State::Won)
    }

    /// Turns the snake, after any turns already waiting. It can't turn back
    /// on itself. The first turn starts it moving.
    pub fn turn(&mut self, dir: Dir) {
        match self.state {
            State::Ready => {
                if dir != self.heading.opposite() {
                    self.heading = dir;
                    self.state = State::Playing;
                }
            }
            State::Playing => {
                let last = self.turns.back().copied().unwrap_or(self.heading);
                if dir != last && dir != last.opposite() && self.turns.len() < QUEUED_TURNS {
                    self.turns.push_back(dir);
                }
            }
            State::Lost | State::Won => {}
        }
    }

    /// How long between moves: a little quicker with each bite.
    fn interval(&self) -> Duration {
        Duration::from_millis(160u64.saturating_sub(2 * u64::from(self.score)).max(70))
    }

    /// When the snake next moves, for the event loop to wake then.
    pub fn next_step(&self) -> Option<Instant> {
        self.next_step.filter(|_| self.state == State::Playing)
    }

    /// Moves the snake if it's time at `now`. `columns` is how many months
    /// the year view shows side by side, for moving up and down.
    pub fn tick(&mut self, now: Instant, columns: u32) {
        if self.state != State::Playing {
            return;
        }
        match self.next_step {
            // Just started: the first move comes one interval from now.
            None => self.next_step = Some(now + self.interval()),
            Some(at) if now >= at => {
                self.step(columns);
                self.next_step = Some(now + self.interval());
            }
            Some(_) => {}
        }
    }

    /// Moves the snake one day on.
    pub fn step(&mut self, columns: u32) {
        if self.state != State::Playing {
            return;
        }
        if let Some(turn) = self.turns.pop_front() {
            self.heading = turn;
        }
        let head = self.head();
        let next = match self.heading {
            Dir::Left => step_across(head, false),
            Dir::Right => step_across(head, true),
            Dir::Up => step_down(head, false, columns),
            Dir::Down => step_down(head, true, columns),
        };
        let Some(next) = next.filter(|day| day.year() == self.year) else {
            self.state = State::Lost;
            return;
        };
        let eats = self.food == Some(next);
        // The tail moves off its day as the head moves on, unless it grows.
        let tail = *self.body.back().expect("the snake has a head");
        if self.occupied.contains(&next) && (eats || next != tail) {
            self.state = State::Lost;
            return;
        }
        if !eats {
            self.body.pop_back();
            self.occupied.remove(&tail);
        }
        self.body.push_front(next);
        self.occupied.insert(next);
        if eats {
            self.score += 1;
            self.place_food();
        }
    }

    /// Puts the food on a random day the snake isn't on. With none left,
    /// the snake has filled the year and won.
    fn place_food(&mut self) {
        let first = NaiveDate::from_ymd_opt(self.year, 1, 1).expect("valid year");
        let days: Vec<NaiveDate> = first
            .iter_days()
            .take_while(|day| day.year() == self.year)
            .filter(|day| !self.occupied.contains(day))
            .collect();
        if days.is_empty() {
            self.food = None;
            self.state = State::Won;
            return;
        }
        let index = (self.random() % days.len() as u64) as usize;
        self.food = Some(days[index]);
    }

    /// xorshift64: plenty for placing food.
    fn random(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }

    /// A seed for the next game, from this one's.
    pub fn next_seed(&mut self) -> u64 {
        self.random()
    }

    #[cfg(test)]
    pub fn set_food(&mut self, day: NaiveDate) {
        self.food = Some(day);
    }

    /// Lays the snake on `days`, its head first, for tests.
    #[cfg(test)]
    pub fn set_body(&mut self, days: &[NaiveDate]) {
        self.body = days.iter().copied().collect();
        self.occupied = days.iter().copied().collect();
    }
}

/// A seed that differs each game, without reading the clock.
pub fn random_seed() -> u64 {
    RandomState::new().hash_one(0u8)
}

/// Every day of `year`.
#[cfg(test)]
fn year_days(year: i32) -> Vec<NaiveDate> {
    let first = NaiveDate::from_ymd_opt(year, 1, 1).unwrap();
    first.iter_days().take_while(|day| day.year() == year).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// A snake on 5 to 7 October 2026 (Monday to Wednesday), heading right,
    /// with its food out of the way on 1 January.
    fn october() -> Snake {
        let mut snake = Snake::new(date(2026, 10, 7), 42);
        snake.set_food(date(2026, 1, 1));
        snake
    }

    fn days(snake: &Snake) -> Vec<NaiveDate> {
        snake.body.iter().copied().collect()
    }

    #[test]
    fn starts_three_days_long_waiting_for_a_key() {
        let mut snake = october();
        assert_eq!(days(&snake), [date(2026, 10, 7), date(2026, 10, 6), date(2026, 10, 5)]);
        assert_eq!(snake.state, State::Ready);
        snake.step(3);
        assert_eq!(snake.head(), date(2026, 10, 7));
        // It can't start by turning back into itself.
        snake.turn(Dir::Left);
        assert_eq!(snake.state, State::Ready);
        snake.turn(Dir::Right);
        assert_eq!(snake.state, State::Playing);
    }

    #[test]
    fn a_snake_at_the_start_of_the_year_is_shorter() {
        let snake = Snake::new(date(2026, 1, 2), 1);
        assert_eq!(days(&snake), [date(2026, 1, 2), date(2026, 1, 1)]);
    }

    #[test]
    fn moves_along_the_row_then_into_the_next_month() {
        let mut snake = october();
        snake.turn(Dir::Right);
        for _ in 0..4 {
            snake.step(3);
        }
        // Along to Sunday 11 October, the end of its row.
        assert_eq!(days(&snake), [date(2026, 10, 11), date(2026, 10, 10), date(2026, 10, 9)]);
        // Then November's second row, which starts on Monday 2.
        snake.step(3);
        assert_eq!(snake.head(), date(2026, 11, 2));
        assert_eq!(snake.state, State::Playing);
    }

    #[test]
    fn moves_down_the_weekday_and_into_the_month_below() {
        let mut snake = october();
        snake.turn(Dir::Down);
        snake.step(3);
        assert_eq!(snake.head(), date(2026, 10, 14));
        snake.step(3);
        snake.step(3);
        assert_eq!(snake.head(), date(2026, 10, 28));
        // Below October, with three months across, is January 2027: a wall.
        snake.step(3);
        assert_eq!(snake.state, State::Lost);
        // With four across, it's still a wall: February 2027.
        let mut snake = october();
        snake.turn(Dir::Down);
        for _ in 0..4 {
            snake.step(4);
        }
        assert_eq!(snake.state, State::Lost);
    }

    #[test]
    fn the_ends_of_the_year_are_walls() {
        let mut snake = Snake::new(date(2026, 12, 30), 1);
        snake.set_food(date(2026, 6, 1));
        snake.turn(Dir::Right);
        snake.step(3);
        assert_eq!(snake.head(), date(2026, 12, 31));
        snake.step(3);
        assert_eq!(snake.state, State::Lost);
        // The head stays where it was, for the game over screen.
        assert_eq!(snake.head(), date(2026, 12, 31));

        let mut snake = Snake::new(date(2026, 1, 3), 1);
        snake.set_food(date(2026, 6, 1));
        snake.turn(Dir::Up);
        snake.step(3);
        assert_eq!(snake.state, State::Lost);
    }

    #[test]
    fn eating_grows_the_snake_scores_and_moves_the_food() {
        let mut snake = october();
        snake.set_food(date(2026, 10, 8));
        snake.turn(Dir::Right);
        snake.step(3);
        assert_eq!(snake.score, 1);
        assert_eq!(days(&snake), [date(2026, 10, 8), date(2026, 10, 7), date(2026, 10, 6), date(2026, 10, 5)]);
        let food = snake.food.unwrap();
        assert_eq!(food.year(), 2026);
        assert_eq!(snake.cell(food), Cell::Food);
        // The next move doesn't grow it.
        snake.step(3);
        assert_eq!(snake.body.len(), 4);
    }

    #[test]
    fn food_is_never_placed_on_the_snake() {
        for seed in 0..200 {
            let mut snake = Snake::new(date(2026, 10, 7), seed);
            // Leave just two days free.
            let mut all = year_days(2026);
            all.retain(|day| *day != date(2026, 3, 3) && *day != date(2026, 8, 8));
            snake.set_body(&all);
            snake.place_food();
            assert!(matches!(snake.food, Some(day) if day == date(2026, 3, 3) || day == date(2026, 8, 8)));
        }
    }

    #[test]
    fn it_cant_turn_back_on_itself_and_keeps_two_quick_turns() {
        let mut snake = october();
        snake.turn(Dir::Right);
        snake.turn(Dir::Left);
        snake.step(3);
        assert_eq!(snake.head(), date(2026, 10, 8));
        // Down then left, typed before the next move, make a U-turn.
        snake.turn(Dir::Down);
        snake.turn(Dir::Left);
        snake.turn(Dir::Up);
        snake.step(3);
        snake.step(3);
        assert_eq!(snake.head(), date(2026, 10, 14));
        // The third turn waited for room and was dropped.
        snake.step(3);
        assert_eq!(snake.head(), date(2026, 10, 13));
    }

    #[test]
    fn running_into_itself_loses_but_its_tail_moves_out_of_the_way() {
        // A snake five long in a ring: 6, 13, 12, 5 and the head on 7.
        let mut snake = october();
        snake.set_body(&[date(2026, 10, 7), date(2026, 10, 6), date(2026, 10, 13), date(2026, 10, 12), date(2026, 10, 5)]);
        snake.turn(Dir::Down);
        snake.step(3);
        snake.turn(Dir::Left);
        // 14 to 13 hits the body.
        snake.step(3);
        assert_eq!(snake.state, State::Lost);

        // Chasing its own tail is fine: 6, 13, 12 and the head on 5, so
        // moving right to 6, where the tail is, works.
        let mut snake = october();
        snake.set_body(&[date(2026, 10, 5), date(2026, 10, 12), date(2026, 10, 13), date(2026, 10, 6)]);
        snake.turn(Dir::Right);
        snake.step(3);
        assert_eq!(snake.state, State::Playing);
        assert_eq!(snake.head(), date(2026, 10, 6));
    }

    #[test]
    fn filling_the_year_wins() {
        let mut snake = october();
        // Everything but 31 December, with the head on the 30th.
        let mut all = year_days(2026);
        all.reverse();
        all.remove(0);
        snake.set_body(&all);
        snake.set_food(date(2026, 12, 31));
        snake.turn(Dir::Right);
        snake.step(3);
        assert_eq!(snake.state, State::Won);
        assert_eq!(snake.food, None);
        assert!(snake.over());
    }

    #[test]
    fn it_moves_on_a_timer_quicker_as_it_eats() {
        let start = Instant::now();
        let mut snake = october();
        snake.tick(start, 3);
        assert_eq!(snake.next_step(), None, "it waits for a key");
        snake.turn(Dir::Right);
        snake.tick(start, 3);
        assert_eq!(snake.next_step(), Some(start + Duration::from_millis(160)));
        snake.tick(start + Duration::from_millis(100), 3);
        assert_eq!(snake.head(), date(2026, 10, 7));
        snake.tick(start + Duration::from_millis(160), 3);
        assert_eq!(snake.head(), date(2026, 10, 8));
        snake.score = 10;
        snake.tick(start + Duration::from_millis(400), 3);
        assert_eq!(snake.next_step(), Some(start + Duration::from_millis(540)));
        snake.score = 100;
        assert_eq!(snake.interval(), Duration::from_millis(70));
    }

    #[test]
    fn nothing_moves_once_its_over() {
        let mut snake = Snake::new(date(2026, 12, 31), 1);
        snake.turn(Dir::Right);
        snake.step(3);
        assert_eq!(snake.state, State::Lost);
        snake.turn(Dir::Up);
        snake.step(3);
        assert_eq!(snake.head(), date(2026, 12, 31));
        assert_eq!(snake.next_step(), None);
    }
}
