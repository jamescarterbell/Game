use rand::thread_rng;
use rand::seq::SliceRandom;
use rand::Rng;
use std::collections::HashSet;
use std::iter;
use std::cmp;
use rs_poker::core::{Rank, Card, Value, Suit, Hand, Rankable};
use std::alloc::handle_alloc_error;
use serde::{Deserialize, Serialize};

use std::error::Error;
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::path::Path;

use std::net::{TcpListener, TcpStream};
use std::cmp::min;
use std::ops::Add;

pub struct PokerGame{
    deck: Vec<Card>,
    pub players: Vec<PokerPlayer>,
    table: Vec<Card>,
    kicked_players: Vec<PokerPlayer>,
}

impl PokerGame{
    pub fn new() -> Self{
        PokerGame{
            deck: vec![],
            players: vec![],
            table: vec![],
            kicked_players: vec![],
        }
    }

    pub fn add_player(&mut self, stream: TcpStream){
        let mut rng = rand::thread_rng();
        self.players.push(PokerPlayer{
            name: rng.gen(),
            money: 1000,
            hand: Vec::<Card>::new(),
            connection: stream,
        })
    }

    pub fn remove_player(&mut self, player: usize){
        self.players.remove(player);
    }

    pub fn issue_hands(&mut self){
        if self.deck.len() < self.players.len() * 2{
            self.deck.append(&mut create_deck());
        }
        for player in &mut self.players{
            player.hand.push(self.deck.pop().unwrap());
            player.hand .push(self.deck.pop().unwrap());
        }
    }

    pub fn clear_hands(&mut self){
        for player in &mut self.players{
            player.hand = Vec::<Card>::new();
        }
    }

    pub fn run_game_round(&mut self) -> GameStatus{
        let mut rng = rand::thread_rng();

        self.clear_hands();
        self.issue_hands();

        let mut out_of_play = HashSet::<usize>::new();
        let mut bets: Vec<u128> = iter::repeat(0).take(self.players.len()).collect();

        for (i, player) in self.players.iter_mut().enumerate(){
            bets[i] += min(player.money, 100);
        }

        self.table = vec![];

        let mut new_cards = self.draw_cards(3);
        self.table.append(&mut new_cards);
        println!("Drawing card!");
        while out_of_play.len() <= self.players.len() - 2{
            if let Err(e) = self.betting(&mut out_of_play, &mut bets){
                return e;
            }
            if self.table.len() < 5{
                let mut new_cards = self.draw_cards(1);
                self.table.append(&mut new_cards);
            }
            else { break; }
        }
        println!("Finding winner!");

        let player_hands: Vec<(u128, (SerdeCard, SerdeCard))> = self.players.iter()
            .map(|x| (x.name,(SerdeCard::from_card(&x.hand[0]), SerdeCard::from_card(&x.hand[1]))))
            .collect();

        let winner = self.find_winner(&mut out_of_play);
        for i in 0..self.players.len(){
            self.players[i].money -= std::cmp::min(self.players[i].money, bets[i]);
        }
        self.players[winner].money += bets.iter().sum::<u128>();

        println!("Winner is: {}", winner);

        
        for (i, player) in self.players.iter_mut().enumerate(){
            player.send_hands(&player_hands);
            player.send_victory(if i == winner {VictoryStatus::Win(player.money)} else {VictoryStatus::Lose(player.money)});
        }

        let mut players_to_kick = Vec::<PokerPlayer>::new();
        let mut players_to_keep = Vec::<PokerPlayer>::new();
        for mut player in self.players.drain(..){
            if player.money > 0{
                println!("{}", player.money);
                players_to_keep.push(player);
                println!("Keeping!");
            }
            else{
                players_to_kick.push(player);;
                println!("Kicking!");
            }
        }
        self.players = players_to_keep;
        if self.players.len() == 1{
            players_to_kick.push(self.players.pop().unwrap());
        }

        for player in players_to_kick.drain(..) {
            self.kicked_players.push(player);
        }

        if self.players.len() > 1{
            println!("{}", self.players.len());
            GameStatus::Running
        }
        else{
            println!("Game finished!");
            GameStatus::Finished
        }
    }

    pub fn find_winner(&mut self, out_of_play: &mut HashSet<usize>) -> usize{

        let mut winner = 0;
        let mut winning_rank = None;
        for i in 0..self.players.len(){
            if out_of_play.contains(&i){
                continue;
            }
            let mut value = get_value(&self.table, &mut self.players[i].hand);
            match winning_rank{
                Some(rank) => {
                    if value > rank {
                        winning_rank = Some(value);
                        winner = i;
                    }
                    else{
                        winning_rank = Some(rank);
                    }
                },
                None => {
                    winning_rank = Some(value);
                    winner = i;
                }
            }
        }
        winner
    }

    pub fn draw_cards(&mut self, amount: usize) -> Vec<Card>{
        let mut cards = vec![];
        for i in 0..amount{
            if self.deck.len() < 1{
                self.deck.append(&mut create_deck());
            }
            cards.push(self.deck.pop().unwrap());
        }
        cards
    }

    pub fn betting(&mut self, out_of_play: &mut HashSet<usize>, bets: &mut Vec<u128>) -> Result<(), GameStatus>{
        let mut rng = rand::thread_rng();
        let mut better: usize = rng.gen::<usize>() % self.players.len();
        let mut actions = Vec::<(u128, PokerActions)>::new();

        while out_of_play.contains(&better){
            better = rng.gen::<usize>() % self.players.len();
        }
        let mut first_better = better;
        let mut raised = true;
        while raised{

            if better == first_better && actions.len() > 1{
                raised = false;
                println!("No one has raised");
            }

            let action_info = ActionInfo{
                position: better as u8,
                table: self.table.iter().map(|x| SerdeCard::from_card(x)).collect(),
                hand: self.players[better].hand.iter().map(|x| SerdeCard::from_card(x)).collect(),
                bets: bets.iter().copied().collect(),
                past_actions: actions.iter().copied().collect()
            };
            println!("Getting player {} actions", better);
            match self.players[better].get_action(action_info){
                    PokerActions::Fold => {
                            out_of_play.insert(better);
                            actions.push((self.players[better].name, PokerActions::Fold));
                            println!("Player {} Folded", better);
                        }
                    PokerActions::Call => {
                            bets[better] = cmp::min(*bets.iter().max().unwrap(), self.players[better].money);
                            actions.push((self.players[better].name, PokerActions::Call));
                        println!("Player {} Called", better);
                        }
                    PokerActions::Raise( mut amount) =>{
                        if amount > * bets.iter().max().unwrap(){
                            raised = true;
                            first_better = better;
                            bets[better] = amount;
                        }
                        else{
                            bets[better] = cmp::min(amount, self.players[better].money);
                        }
                        actions.push((self.players[better].name, PokerActions::Raise(amount)));
                        println!("Player {} Raised", better);
                    }
                    PokerActions::Disconnected =>{
                        return Err(GameStatus::Error);
                    }
                }
            better = (better + 1) % self.players.len();
            if out_of_play.len() == self.players.len() - 1{
                println!("Went through all players");
                break;
            }
            while out_of_play.contains(&better){
                println!("Looking for new next");
                better = (better + 1)% self.players.len();
            }
        }
        Ok(())
    }

    pub fn kick_players(&mut self) -> Vec<TcpStream>{
        let mut kick = Vec::<TcpStream>::new();
        for mut player in self.kicked_players.drain(..){
            player.send_victory(if player.money == 0 {VictoryStatus::Lose(0)} else {VictoryStatus::FinalWin(player.money)});
            kick.push(player.connection);
        }
        kick
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub enum VictoryStatus{
    FinalWin(u128),
    Win(u128),
    Lose(u128),
}

pub enum GameStatus{
    Running,
    Error,
    Finished,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ActionInfo{
    position: u8,
    table: Vec<SerdeCard>,
    hand: Vec<SerdeCard>,
    bets: Vec<u128>,
    past_actions: Vec<(u128, PokerActions)>
}

pub struct PokerPlayer{
    name: u128,
    pub money: u128,
    hand: Vec<Card>,
    pub connection: TcpStream,
}

impl PokerPlayer{
    pub fn get_action(&mut self, action_info: ActionInfo) -> PokerActions{
        let json = serde_json::to_string(&action_info).unwrap();

        self.send_message(json);

        let mut de = serde_json::Deserializer::from_reader(self.connection.try_clone().unwrap());
        match PokerActionsCards::deserialize(&mut de){
            Ok(action) => {
                println!("Hello");
                println!("Got Action!");
                if let Some(cards) = action.cards{
                    self.hand = vec![cards[0].to_card(), cards[1].to_card()];
                }
                action.action
            },
            Err(e) => PokerActions::Disconnected
        }
    }

    pub fn send_victory(&mut self, victory: VictoryStatus){

        let json = serde_json::to_string(&victory).unwrap();

        self.send_message(json);
    }

    pub fn send_hands(&mut self, hands: &Vec<(u128, (SerdeCard, SerdeCard))>){
        let json = serde_json::to_string(hands).unwrap();
        let json = String::from("Hands: ").add(&json);
        self.send_message(json);
    }

    fn send_message(&mut self, message: String){
        let message = message.as_bytes();
        while let Err(e) = self.connection.write(&(message.len() as u32).to_be_bytes()) {
            self.connection.write(&(message.len() as u32).to_be_bytes());
        };
        self.connection.write(message);
    }
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone)]
pub enum PokerActions{
    Raise(u128),
    Fold,
    Call,
    Disconnected
}

#[derive(Serialize, Deserialize, Debug)]
pub struct PokerActionsCards{
    pub action: PokerActions,
    pub cards: Option<Vec<SerdeCard>>
}

fn create_deck() -> Vec<Card>{
    let mut deck = vec![];
    for i in 0..13{
        deck.push(Card{
            value: num_to_value(i),
            suit: Suit::Diamond,
        });
        deck.push(Card{
            value: num_to_value(i),
            suit: Suit::Spade,
        });
        deck.push(Card{
            value: num_to_value(i),
            suit: Suit::Heart,
        });
        deck.push(Card{
            value: num_to_value(i),
            suit: Suit::Club,
        });
    };
    let mut rng = thread_rng();
    deck.shuffle(&mut rng);
    deck.shuffle(&mut rng);
    deck.shuffle(&mut rng);
    deck.shuffle(&mut rng);
    deck
}

fn num_to_value(num: usize) -> Value{
    match num{
        0 => Value::Two,
        1 => Value::Three,
        2 => Value::Four,
        3 => Value::Five,
        4 => Value::Six,
        5 => Value::Seven,
        6 => Value::Eight,
        7 => Value::Nine,
        8 => Value::Ten,
        9 => Value::Jack,
        10 => Value::Queen,
        11 => Value::King,
        12 => Value::Ace,
        _ => Value::Two,
    }
}

fn value_to_num(num: Value) -> u8{
    match num{
        Value::Two => 0,
        Value::Three => 1,
        Value::Four => 2,
        Value::Five => 3,
        Value::Six => 4,
        Value::Seven => 5,
        Value::Eight => 6,
        Value::Nine => 7,
        Value::Ten => 8,
        Value::Jack => 9,
        Value::Queen => 10,
        Value::King => 11,
        Value::Ace => 12,
        _ => 0,
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SerdeCard{
    value: u8,
    suit: SerdeSuit,
}

impl SerdeCard{
    pub fn from_card(card: &Card) -> SerdeCard{
        SerdeCard{
            value: value_to_num(card.value),
            suit: match card.suit{
                Suit::Spade => SerdeSuit::Spade,
                Suit::Club => SerdeSuit::Club,
                Suit::Heart => SerdeSuit::Heart,
                Suit::Diamond => SerdeSuit::Diamond,
            }
        }
    }

    pub fn to_card(&self) -> Card{
        Card{
            value: num_to_value(self.value as usize),
            suit: match self.suit{
                SerdeSuit::Spade => Suit::Spade,
                SerdeSuit::Club => Suit::Club,
                SerdeSuit::Heart => Suit::Heart,
                SerdeSuit::Diamond => Suit::Diamond,
            }
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
enum SerdeSuit{
    Heart,
    Diamond,
    Spade,
    Club
}

fn get_value(table: &Vec<Card>,hand: &mut Vec<Card>) -> Rank{
    let mut new_table = table.iter().copied().collect();
    let mut hand_eval = Hand::new_with_cards(new_table);
    hand_eval.push(hand[0].clone());
    hand_eval.push(hand[1].clone());
    while hand_eval.len() < 5{
        hand_eval.push(Card{value: Value::Two, suit: Suit::Heart});
    }
    hand.rank()
}
