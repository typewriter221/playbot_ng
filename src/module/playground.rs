use crate::module::prelude::*;
use playground::{self, ExecuteRequest, Channel, Mode, CrateType};
use regex::Regex;
use futures::future::LocalFutureObj;
use futures::prelude::*;

lazy_static! {
    static ref CRATE_ATTRS: Regex = Regex::new(r"^(\s*#!\[.*?\])*").unwrap();
}

pub(crate) enum Playground {}

impl Module for Playground {
    fn init(commands: &mut CommandRegistry) {
        commands.add_fallback_handler(playground_handler);
    }
}

fn playground_handler<'a>(ctx: &'a Context) -> LocalFutureObj<'a, Flow> {
    LocalFutureObj::new(async move {
        if !ctx.is_directly_addressed() {
            return Flow::Continue;
        }

        let mut request = ExecuteRequest::new("");
        let mut body = ctx.body();
        let mut bare = false;

        // Parse flags
        loop {
            body = body.trim_left();
            let flag = body.split_whitespace().next().unwrap_or("");

            match flag {
                "--stable" => request.set_channel(Channel::Stable),
                "--beta" => request.set_channel(Channel::Beta),
                "--nightly" => request.set_channel(Channel::Nightly),
                "--version" | "VERSION" => {
                    await!(print_version(request.channel(), &ctx));
                    return Flow::Break;
                },
                "--bare" | "--mini" => bare = true,
                "--debug" => request.set_mode(Mode::Debug),
                "--release" => request.set_mode(Mode::Release),
                "--bin" => request.set_crate_type(CrateType::Bin),
                "--lib" => request.set_crate_type(CrateType::Lib),
                "--2015" => request.set_edition(Some("2015".to_owned())),
                "--2018" => request.set_edition(Some("2018".to_owned())),
                "help" | "h" | "-h" | "-help" | "--help" | "--h" => {
                    super::help::display_help(ctx);
                    return Flow::Break;
                },
                "--" => {
                    body = &body[flag.len()..];
                    break;
                },
                _ => break,
            }

            body = &body[flag.len()..];
        }

        body = body.trim_left();

        let code = if bare { body.to_string() } else {
            let crate_attrs = CRATE_ATTRS.find(body)
                .map(|attr| attr.as_str())
                .unwrap_or("");

            let body_code = &body[crate_attrs.len()..];

            format!(include_str!("../../template.rs"),
                crate_attrs = crate_attrs,
                code = body_code,
            )
        };

        request.set_code(code);

        await!(execute(&ctx, &request));

        Flow::Break
    }.boxed())
}

fn print_version<'a>(channel: Channel, ctx: &'a Context) -> impl Future<Output = ()> + 'a {
    async move {
        let resp = match await!(playground::async_version(channel)) {
            Err(e) => return eprintln!("Failed to get version: {:?}", e),
            Ok(resp) => resp,
        };

        let version = format!("{version} ({hash:.9} {date})",
            version = resp.version,
            hash = resp.hash,
            date = resp.date,
        );

        ctx.reply(version);
    }
}

pub fn execute<'a>(ctx: &'a Context, request: &'a ExecuteRequest) -> impl Future<Output = ()> + 'a {
    async move {
        let resp = match await!(playground::async_execute(&request)) {
            Ok(resp) => resp,
            Err(e) => return {
                eprintln!("Failed to execute code: {:?}", e);
            },
        };

        let output = if resp.success { &resp.stdout } else { &resp.stderr };
        let take_count = if resp.success { 2 } else { 1 };
        let lines = output
            .lines()
            .filter(|line| {
                if resp.success {
                    return true;
                }

                !line.trim().starts_with("Compiling")
                && !line.trim().starts_with("Finished")
                && !line.trim().starts_with("Running")
            })
            .take(take_count);

        for line in lines {
            ctx.reply(line);
        }

        if output.lines().count() > take_count {
            let code = format!(include_str!("../../paste_template.rs"),
                code = request.code(),
                stdout = resp.stdout,
                stderr = resp.stderr,
            );

            let url = match await!(playground::async_paste(code, request.channel(), request.mode())) {
                Ok(url) => url,
                Err(e) => return {
                    eprintln!("Failed to paste code: {:?}", e);
                },
            };

            ctx.reply(format!("~~~ Full output: {}", url));
        }
    }
}
