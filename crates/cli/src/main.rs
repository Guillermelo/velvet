use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use std::{
    io::{self, Write},
    path::PathBuf,
    sync::Arc,
};
use velvet_audio::{MediaCache, Player};
use velvet_core::{Command, Effect, Project, Session};

#[derive(Parser)]
#[command(
    name = "velvet",
    version,
    about = "A small, command-driven audio workstation"
)]
struct Cli {
    #[arg(long, global = true, default_value = ".")]
    project: PathBuf,
    #[command(subcommand)]
    action: Action,
}
#[derive(Subcommand)]
enum Action {
    New {
        directory: PathBuf,
        #[arg(long)]
        name: Option<String>,
    },
    Inspect,
    Save,
    Undo,
    Redo,
    History,
    Track {
        #[command(subcommand)]
        action: TrackAction,
    },
    Clip {
        #[command(subcommand)]
        action: ClipAction,
    },
    Device {
        #[command(subcommand)]
        action: DeviceAction,
    },
    Tempo {
        bpm: f64,
    },
    Master {
        #[arg(allow_hyphen_values = true)]
        volume_db: f64,
    },
    Render {
        path: PathBuf,
    },
    Play {
        #[arg(long, default_value = "0")]
        seek: f64,
    },
    Ask {
        prompt: String,
    },
    /// Persistent transport session. Type JSON commands, undo, redo, inspect or quit.
    Shell,
    /// Execute any serialized command used by the GUI and AI.
    Command {
        json: String,
    },
}
#[derive(Subcommand)]
enum TrackAction {
    List,
    Add {
        #[arg(long)]
        name: String,
    },
    Delete {
        track_id: String,
    },
    Rename {
        track_id: String,
        name: String,
    },
    Volume {
        track_id: String,
        #[arg(allow_hyphen_values = true)]
        volume_db: f64,
    },
    Pan {
        track_id: String,
        #[arg(allow_hyphen_values = true)]
        pan: f64,
    },
    Mute {
        track_id: String,
        value: bool,
    },
    Solo {
        track_id: String,
        value: bool,
    },
}
#[derive(Subcommand)]
enum ClipAction {
    List {
        track_id: String,
    },
    Import {
        #[arg(long)]
        track: String,
        path: PathBuf,
        #[arg(long, default_value = "0")]
        beats: f64,
    },
    Move {
        track_id: String,
        clip_id: String,
        beats: f64,
    },
    Trim {
        track_id: String,
        clip_id: String,
        offset: f64,
        length: f64,
    },
    Delete {
        track_id: String,
        clip_id: String,
    },
    Relink {
        track_id: String,
        clip_id: String,
        path: PathBuf,
    },
}
#[derive(Subcommand)]
enum DeviceAction {
    List {
        track_id: String,
    },
    Add {
        track_id: String,
        kind: String,
    },
    Remove {
        track_id: String,
        device_id: String,
    },
    Set {
        track_id: String,
        device_id: String,
        parameter: String,
        #[arg(allow_hyphen_values = true)]
        value: f64,
    },
}
fn main() {
    if let Err(e) = run() {
        eprintln!("Velvet: {e:#}");
        std::process::exit(1);
    }
}
fn persist(s: &mut Session) -> Result<()> {
    s.execute(Command::SaveProject)?;
    velvet_core::atomic_write(
        &s.root.join(".velvet-history.json"),
        &serde_json::to_vec(&(s.history.clone(), s.redo_history.clone()))?,
    )
}
fn run() -> Result<()> {
    let cli = Cli::parse();
    if let Action::New { directory, name } = cli.action {
        anyhow::ensure!(
            !directory.join("project.yaml").exists(),
            "A project already exists there"
        );
        let name = name.unwrap_or_else(|| {
            directory
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into()
        });
        velvet_core::save(&directory, &Project::new(name))?;
        println!("Created {}", directory.display());
        return Ok(());
    }
    let mut s = Session::open(&cli.project)?;
    if let Ok(text) = std::fs::read_to_string(s.root.join(".velvet-history.json")) {
        if let Ok((history, redo)) = serde_json::from_str::<(
            Vec<velvet_core::HistoryEntry>,
            Vec<velvet_core::HistoryEntry>,
        )>(&text)
        {
            let expected = history
                .last()
                .map(|h| &h.after)
                .or_else(|| redo.last().map(|h| &h.before));
            if expected == Some(&s.project)
                && history
                    .iter()
                    .chain(&redo)
                    .all(|h| h.before.validate().is_ok() && h.after.validate().is_ok())
            {
                s.history = history;
                s.redo_history = redo;
            }
        }
    }
    let command = match cli.action {
        Action::New { .. } => unreachable!(),
        Action::Inspect => {
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"project":s.project,"missing":s.project.missing(&s.root)})
                )?
            );
            return Ok(());
        }
        Action::History => {
            for h in &s.history {
                println!("{}", h.label);
            }
            return Ok(());
        }
        Action::Undo => {
            anyhow::ensure!(s.undo(), "Nothing to undo");
            persist(&mut s)?;
            return Ok(());
        }
        Action::Redo => {
            anyhow::ensure!(s.redo(), "Nothing to redo");
            persist(&mut s)?;
            return Ok(());
        }
        Action::Save => Command::SaveProject,
        Action::Tempo { bpm } => Command::SetTempo { bpm },
        Action::Master { volume_db } => Command::SetMasterVolume { volume_db },
        Action::Track { action } => match action {
            TrackAction::List => {
                for t in &s.project.tracks {
                    println!(
                        "{}  {}  {:.1} dB  pan {:.2}  mute={} solo={}",
                        t.id, t.name, t.mixer.volume_db, t.mixer.pan, t.mixer.mute, t.mixer.solo
                    );
                }
                return Ok(());
            }
            TrackAction::Add { name } => Command::AddTrack { name },
            TrackAction::Delete { track_id } => Command::RemoveTrack { track_id },
            TrackAction::Rename { track_id, name } => Command::RenameTrack { track_id, name },
            TrackAction::Volume {
                track_id,
                volume_db,
            } => Command::SetTrackVolume {
                track_id,
                volume_db,
            },
            TrackAction::Pan { track_id, pan } => Command::SetTrackPan { track_id, pan },
            TrackAction::Mute { track_id, value } => Command::SetMute {
                track_id,
                mute: value,
            },
            TrackAction::Solo { track_id, value } => Command::SetSolo {
                track_id,
                solo: value,
            },
        },
        Action::Clip { action } => match action {
            ClipAction::List { track_id } => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&s.project.track(&track_id)?.clips)?
                );
                return Ok(());
            }
            ClipAction::Import { track, path, beats } => {
                velvet_audio::import(&mut s, &track, &path, beats)?;
                persist(&mut s)?;
                println!("Imported {}", path.display());
                return Ok(());
            }
            ClipAction::Move {
                track_id,
                clip_id,
                beats,
            } => Command::MoveClip {
                track_id,
                clip_id,
                start_beats: beats,
            },
            ClipAction::Trim {
                track_id,
                clip_id,
                offset,
                length,
            } => Command::TrimClip {
                track_id,
                clip_id,
                offset_seconds: offset,
                length_seconds: length,
            },
            ClipAction::Delete { track_id, clip_id } => Command::RemoveClip { track_id, clip_id },
            ClipAction::Relink {
                track_id,
                clip_id,
                path,
            } => {
                let path = path.canonicalize()?;
                velvet_audio::decode(&path)?;
                Command::RelinkClip {
                    track_id,
                    clip_id,
                    source: velvet_core::Source {
                        path,
                        kind: velvet_core::SourceKind::External,
                    },
                }
            }
        },
        Action::Device { action } => match action {
            DeviceAction::List { track_id } => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(s.project.devices(&track_id)?)?
                );
                return Ok(());
            }
            DeviceAction::Add { track_id, kind } => Command::AddDevice { track_id, kind },
            DeviceAction::Remove {
                track_id,
                device_id,
            } => Command::RemoveDevice {
                track_id,
                device_id,
            },
            DeviceAction::Set {
                track_id,
                device_id,
                parameter,
                value,
            } => Command::SetDeviceParameter {
                track_id,
                device_id,
                parameter,
                value,
            },
        },
        Action::Render { path } => Command::RenderProject { path },
        Action::Command { json } => serde_json::from_str(&json)?,
        Action::Play { seek } => {
            s.execute(Command::Seek { seconds: seek })?;
            play(&s, true)?;
            return Ok(());
        }
        Action::Ask { prompt } => {
            let result = velvet_ai::Agent::from_env()?.ask(&mut s, &prompt);
            // Even if the network fails after applying a tool, preserve undoable changes.
            persist(&mut s)?;
            let a = result?;
            for action in a.actions {
                println!("{action}");
            }
            println!("{}", a.text);
            if s.transport.playing {
                play(&s, true)?;
            }
            return Ok(());
        }
        Action::Shell => return shell(&mut s),
    };
    match s.execute(command)? {
        Effect::Render(path) => {
            let m = velvet_audio::mix(
                &s.project,
                &s.root,
                s.project.audio.sample_rate,
                &mut MediaCache::default(),
            )?;
            velvet_audio::export(&m, &path)?;
            println!("Rendered {} (peak {:.2})", path.display(), m.peak);
        }
        Effect::Transport => bail!("Use velvet shell for persistent play/pause/stop/seek controls"),
        _ => {
            persist(&mut s)?;
            if let Some(t) = s.project.tracks.last() {
                println!("{}  {}", t.id, t.name);
            }
        }
    }
    Ok(())
}
fn prepared(s: &Session) -> Result<Player> {
    let rate = Player::output_rate()?;
    let m = velvet_audio::mix(&s.project, &s.root, rate, &mut MediaCache::default())?;
    for p in &m.missing {
        eprintln!("Missing: {} (silent)", p.display());
    }
    Player::new(Arc::new(m), s.transport.seconds)
}
fn play(s: &Session, wait: bool) -> Result<()> {
    let p = prepared(s)?;
    p.play();
    println!("Playing. Press Enter to stop.");
    if wait {
        let mut text = String::new();
        io::stdin().read_line(&mut text)?;
    }
    Ok(())
}
fn shell(s: &mut Session) -> Result<()> {
    println!("JSON command shell: {{\"op\":\"play\"}}, {{\"op\":\"seek\",\"seconds\":4}}, undo, redo, inspect, quit");
    let mut player: Option<Player> = None;
    loop {
        if let Some(p) = &player {
            s.transport.seconds = p.seconds();
            s.transport.playing = p.playing();
        }
        print!("velvet> ");
        io::stdout().flush()?;
        let mut line = String::new();
        if io::stdin().read_line(&mut line)? == 0 {
            break;
        }
        if let Some(p) = &player {
            s.transport.seconds = p.seconds();
            s.transport.playing = p.playing();
        }
        let result = (|| -> Result<()> {
            match line.trim() {
                "quit" | "exit" => return Ok(()),
                "inspect" => {
                    println!("{}", serde_json::to_string_pretty(&s.project)?);
                    return Ok(());
                }
                "undo" => {
                    s.undo();
                }
                "redo" => {
                    s.redo();
                }
                text => {
                    let command =
                        serde_json::from_str(text).context("Expected a JSON Velvet command")?;
                    if let Effect::Render(path) = s.execute(command)? {
                        let m = velvet_audio::mix(
                            &s.project,
                            &s.root,
                            s.project.audio.sample_rate,
                            &mut MediaCache::default(),
                        )?;
                        velvet_audio::export(&m, &path)?;
                    }
                }
            }
            let playing = s.transport.playing;
            if let Some(p) = &player {
                p.pause();
                p.seek(s.transport.seconds);
            }
            if playing {
                let p = prepared(s)?;
                p.play();
                player = Some(p);
            } else {
                player = None;
            }
            persist(s)?;
            Ok(())
        })();
        if let Err(e) = result {
            eprintln!("{e:#}");
        }
        if matches!(line.trim(), "quit" | "exit") {
            break;
        }
    }
    Ok(())
}
