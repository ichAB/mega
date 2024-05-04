use std::str::FromStr;

use clap::Parser;
use sea_orm::{ActiveModelTrait, DbConn, Set};
use venus::{
    hash::SHA1,
    internal::object::{commit::Commit, tree::Tree, types::ObjectType},
};

use crate::{
    command::branch,
    db,
    model::reference::{self, ActiveModel},
    utils::{object_ext::TreeExt, util},
};

use super::{
    load_object,
    restore::{restore_index, restore_worktree},
    status,
};

#[derive(Parser, Debug)]
pub struct SwitchArgs {
    #[clap(required_unless_present("create"), required_unless_present("detach"))]
    branch: Option<String>,

    #[clap(long, short, group = "sub")]
    create: Option<String>,

    //available only with create
    #[clap(requires = "create")]
    create_base: Option<String>,

    #[clap(long, short, action, default_value = "false", group = "sub")]
    detach: bool,
}

fn get_commit_base(commit_base: &str) -> Result<SHA1, String> {
    let storage = util::objects_storage();

    let commits = storage.search(commit_base);
    if commits.is_empty() {
        return Err(format!("fatal: invalid reference: {}", commit_base));
    } else if commits.len() > 1 {
        return Err(format!("fatal: ambiguous argument: {}", commit_base));
    }
    if storage.is_object_type(&commits[0], ObjectType::Commit) {
        Err(format!("fatal: reference is not a commit: {}", commit_base))
    } else {
        Ok(commits[0])
    }
}

pub async fn execute(args: SwitchArgs) {
    // check status
    let unstaged = status::changes_to_be_staged();
    if !unstaged.deleted.is_empty() || !unstaged.modified.is_empty() {
        status::execute().await;
        eprintln!("fatal: uncommitted changes, can't switch branch");
        return;
    } else if !status::changes_to_be_committed().await.is_empty() {
        status::execute().await;
        eprintln!("fatal: unstaged changes, can't switch branch");
        return;
    }

    let db = db::get_db_conn().await.unwrap();
    match args.create {
        Some(new_branch_name) => {
            branch::create_branch(new_branch_name.clone(), args.create_base).await;
            switch_to_branch(&db, new_branch_name).await;
        }
        None => match args.detach {
            true => {
                let commit_base = get_commit_base(&args.branch.unwrap());
                if commit_base.is_err() {
                    eprintln!("{}", commit_base.unwrap());
                    return;
                }
                switch_to_commit(&db, commit_base.unwrap()).await;
            }
            false => {
                switch_to_branch(&db, args.branch.unwrap()).await;
            }
        },
    }
}

/// change the working directory to the version of commit_hash
async fn switch_to_commit(db: &DbConn, commit_hash: SHA1) {
    restore_to_commit(commit_hash).await;
    // update HEAD
    let mut head: ActiveModel = reference::Model::current_head(db).await.unwrap().into();
    head.name = Set(None);
    head.commit = Set(Some(commit_hash.to_string()));
    head.save(db).await.unwrap();
}

async fn switch_to_branch(db: &DbConn, branch_name: String) {
    let target_branch = reference::Model::find_branch_by_name(db, &branch_name)
        .await
        .unwrap();
    if target_branch.is_none() {
        eprintln!("fatal: branch '{}' not found", &branch_name);
        return;
    }
    let commit_id = target_branch.unwrap().commit.unwrap();
    let commit_id = SHA1::from_str(&commit_id).unwrap();
    restore_to_commit(commit_id).await;
    // update HEAD
    let mut head: ActiveModel = reference::Model::current_head(db).await.unwrap().into();

    head.name = Set(Some(branch_name));
    head.commit = Set(None);
    head.save(db).await.unwrap();
}

async fn restore_to_commit(commit_id: SHA1) {
    let commit = load_object::<Commit>(&commit_id).unwrap();
    let tree_id = commit.tree_id;
    let tree = load_object::<Tree>(&tree_id).unwrap();
    let target_blobs = tree.get_plain_items();
    restore_index(&vec![], &target_blobs);
    restore_worktree(&vec![], &target_blobs);
}
