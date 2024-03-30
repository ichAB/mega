use std::{
    path::{Component, PathBuf},
    sync::mpsc::{self, Receiver},
    vec,
};

use async_trait::async_trait;
use bytes::Bytes;

use callisto::raw_blob;
use common::utils::MEGA_BRANCH_NAME;
use jupiter::{context::Context, storage::batch_query_by_columns};
use mercury::internal::pack::encode::PackEncoder;
use venus::{
    errors::GitError,
    internal::{
        object::{blob::Blob, commit::Commit, tag::Tag, tree::Tree, types::ObjectType},
        pack::{
            entry::Entry,
            reference::{RefCommand, Refs},
        },
    },
    monorepo::mr::MergeRequest,
    repo::Repo,
};

use crate::pack::handler::{check_head_hash, decode_for_receiver, PackHandler};

pub struct MonoRepo {
    pub context: Context,
    pub path: PathBuf,
    pub from_hash: Option<String>,
    pub to_hash: Option<String>,
}

#[async_trait]
impl PackHandler for MonoRepo {
    async fn head_hash(&self) -> (String, Vec<Refs>) {
        let storage = self.context.services.mega_storage.clone();

        let result = storage.get_ref(self.path.to_str().unwrap()).await.unwrap();
        let refs = if result.is_some() {
            vec![result.unwrap().into()]
        } else {
            let target_path = self.path.clone();
            let ref_hash = storage
                .get_ref("/")
                .await
                .unwrap()
                .unwrap()
                .ref_commit_hash
                .clone();

            let commit: Commit = storage
                .get_commit_by_hash(&Repo::empty(), &ref_hash)
                .await
                .unwrap()
                .unwrap()
                .into();
            let tree_id = commit.tree_id.to_plain_str();
            let mut tree: Tree = storage
                .get_tree_by_hash(&Repo::empty(), &tree_id)
                .await
                .unwrap()
                .unwrap()
                .into();

            for component in target_path.components() {
                if component != Component::RootDir {
                    let path_name = component.as_os_str().to_str().unwrap();
                    let sha1 = tree
                        .tree_items
                        .iter()
                        .find(|x| x.name == path_name)
                        .map(|x| x.id);
                    if let Some(sha1) = sha1 {
                        tree = storage
                            .get_tree_by_hash(&Repo::empty(), &sha1.to_plain_str())
                            .await
                            .unwrap()
                            .unwrap()
                            .into();
                    } else {
                        return check_head_hash(vec![]);
                    }
                }
            }

            let c = Commit::from_tree_id(
                tree.id,
                vec![],
                "This commit was generated by mega for maintain refs",
            );
            storage
                .save_ref(
                    self.path.to_str().unwrap(),
                    &c.id.to_plain_str(),
                    &c.tree_id.to_plain_str(),
                )
                .await
                .unwrap();
            storage
                .save_mega_commits(&Repo::empty(), vec![c.clone()])
                .await
                .unwrap();

            vec![Refs {
                ref_name: MEGA_BRANCH_NAME.to_string(),
                ref_hash: c.id.to_plain_str(),
                default_branch: true,
                ..Default::default()
            }]
        };
        check_head_hash(refs)
    }
    // 001e# service=git-upload-pack\n
    // 0000 00b2
    // c9ba5f3b45016391455e70cbbf2db55efeb013f6 HEAD\0
    // shallow deepen-since deepen-not deepen-relative multi_ack_detailed no-done include-tag side-band-64k ofs-delta agent=mega/0.1.0\n
    // 002e c9ba5f3b45016391455e70cbbf2db55efeb013f6 \n0000

    async fn unpack(&self, pack_file: Bytes) -> Result<(), GitError> {
        let receiver = decode_for_receiver(pack_file).unwrap();

        let storage = self.context.services.mega_storage.clone();

        let (mut mr, mr_exist) = self.get_mr().await;

        let mut commit_size = 0;
        if mr_exist {
            if mr.from_hash == self.from_hash.clone().unwrap() {
                let to_hash = self.to_hash.clone().unwrap();
                if mr.to_hash != to_hash {
                    let comment = self.comment_for_force_update(&mr.to_hash, &to_hash);
                    mr.to_hash = to_hash;
                    storage
                        .add_mr_comment(mr.id, 0, Some(comment))
                        .await
                        .unwrap();
                    commit_size = self.save_entry(receiver).await;
                }
            } else {
                mr.close();
                storage
                    .add_mr_comment(mr.id, 0, Some("Mega closed MR due to conflict".to_string()))
                    .await
                    .unwrap();
            }
            storage.update_mr(mr.clone()).await.unwrap();
        } else {
            commit_size = self.save_entry(receiver).await;

            storage.save_mr(mr.clone()).await.unwrap();
        };

        if commit_size > 1 {
            mr.close();
            storage
                .add_mr_comment(
                    mr.id,
                    0,
                    Some("Mega closed MR due to multi commit detected".to_string()),
                )
                .await
                .unwrap();
        }
        Ok(())
    }

    async fn full_pack(&self) -> Result<Vec<u8>, GitError> {
        let (sender, receiver) = mpsc::channel();
        let repo = &Repo::empty();
        let storage = self.context.services.mega_storage.clone();
        let obj_num = storage.get_obj_count_by_repo_id(repo).await;
        let mut encoder = PackEncoder::new(obj_num, 0);

        for m in storage
            .get_commits_by_repo_id(repo)
            .await
            .unwrap()
            .into_iter()
        {
            let c: Commit = m.into();
            let entry: Entry = c.into();
            sender.send(entry).unwrap();
        }

        for m in storage
            .get_trees_by_repo_id(repo)
            .await
            .unwrap()
            .into_iter()
        {
            let c: Tree = m.into();
            let entry: Entry = c.into();
            sender.send(entry).unwrap();
        }

        let bids: Vec<String> = storage
            .get_blobs_by_repo_id(repo)
            .await
            .unwrap()
            .into_iter()
            .map(|b| b.blob_id)
            .collect();

        let raw_blobs = batch_query_by_columns::<raw_blob::Entity, raw_blob::Column>(
            storage.get_connection(),
            raw_blob::Column::Sha1,
            bids,
            None,
            None,
        )
        .await
        .unwrap();

        for m in raw_blobs {
            // todo handle storage type
            let c: Blob = m.into();
            let entry: Entry = c.into();
            sender.send(entry).unwrap();
        }

        for m in storage.get_tags_by_repo_id(repo).await.unwrap().into_iter() {
            let c: Tag = m.into();
            let entry: Entry = c.into();
            sender.send(entry).unwrap();
        }
        drop(sender);
        let data = encoder.encode(receiver).unwrap();

        Ok(data)
    }

    async fn check_commit_exist(&self, hash: &str) -> bool {
        self.context
            .services
            .mega_storage
            .get_commit_by_hash(&Repo::empty(), hash)
            .await
            .unwrap()
            .is_some()
    }

    async fn incremental_pack(
        &self,
        _want: Vec<String>,
        _have: Vec<String>,
    ) -> Result<Vec<u8>, GitError> {
        todo!()
    }

    async fn update_refs(&self, _: &RefCommand) -> Result<(), GitError> {
        //do nothing in monorepo because need mr to handle refs
        Ok(())
    }

    async fn check_default_branch(&self) -> bool {
        true
    }
}

impl MonoRepo {
    async fn get_mr(&self) -> (MergeRequest, bool) {
        let storage = self.context.services.mega_storage.clone();

        let mr = storage
            .get_open_mr(self.path.to_str().unwrap())
            .await
            .unwrap();
        if let Some(mr) = mr {
            (mr, true)
        } else {
            let mr = MergeRequest {
                path: self.path.to_str().unwrap().to_owned(),
                from_hash: self.from_hash.clone().unwrap(),
                to_hash: self.to_hash.clone().unwrap(),
                ..Default::default()
            };
            (mr, false)
        }
    }

    fn comment_for_force_update(&self, from: &str, to: &str) -> String {
        format!(
            "Mega updated the mr automatic from {} to {}",
            &from[..6],
            &to[..6]
        )
    }

    async fn save_entry(&self, receiver: Receiver<Entry>) -> i32 {
        let storage = self.context.services.mega_storage.clone();
        let mut entry_list = Vec::new();

        let mut commit_size = 0;
        for entry in receiver {
            if entry.obj_type == ObjectType::Commit {
                commit_size += 1;
            }
            entry_list.push(entry);
            if entry_list.len() >= 1000 {
                storage.save_entry(entry_list).await.unwrap();
                entry_list = Vec::new();
            }
        }
        storage.save_entry(entry_list).await.unwrap();
        commit_size
    }
}
