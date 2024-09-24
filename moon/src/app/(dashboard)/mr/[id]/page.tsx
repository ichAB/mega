'use client'
import { useEffect, useState } from "react";
import { Card, Button, List, Tabs, TabsProps, Space, Timeline } from 'antd/lib';
import { useRouter } from 'next/navigation';
import { CommentOutlined, MergeOutlined } from '@ant-design/icons';
import { formatDistance, fromUnixTime } from 'date-fns';

interface MRDetail {
    status: string,
    conversions: Conversation[],
    title: string,
}
interface Conversation {
    user_id: number,
    conv_type: String,
    comment: String,
    created_at: number,
}

export default function MRDetailPage({ params }: { params: { id: string } }) {
    const [mrDetail, setMrDetail] = useState<MRDetail>(
        {
            status: "", 
            conversions: [], 
            title: "",
          }
    );
    const router = useRouter();
    const [filedata, setFileData] = useState([]);
    const [loadings, setLoadings] = useState<boolean[]>([]);

    useEffect(() => {
        const fetchFileList = async () => {
            set_to_loading(2)
            try {
                const detail = await fetch(`/api/mr/${params.id}/detail`);
                const detail_json = await detail.json();
                setMrDetail(detail_json.data.data);
                const res = await fetch(`/api/mr/${params.id}/files`);
                const result = await res.json();
                setFileData(result.data.data);
            } finally {
                cancel_loading(2)
            }
        };
        fetchFileList();
    }, [params.id]);

    const set_to_loading = (index: number) => {
        setLoadings((prevLoadings) => {
            const newLoadings = [...prevLoadings];
            newLoadings[index] = true;
            return newLoadings;
        });
    }

    const cancel_loading = (index: number) => {
        setLoadings((prevLoadings) => {
            const newLoadings = [...prevLoadings];
            newLoadings[index] = false;
            return newLoadings;
        });
    }

    const approve_mr = async (index: number, mr_link: string) => {
        set_to_loading(index);
        const res = await fetch(`/api/mr/${mr_link}/merge`, {
            method: 'POST',
        });
        if (res) {
            cancel_loading(index);
        }
    };

    let conv_items = mrDetail?.conversions.map(conv => {
        let icon;
        let children;
        switch (conv.conv_type) {
            case "Comment": icon = <CommentOutlined />; children = conv.comment; break;
            case "Merged": icon = <MergeOutlined />; children = "Merged via the queue into main " + formatDistance(fromUnixTime(conv.created_at), new Date(), { addSuffix: true }); break;
            // default: icon = <CommentOutlined />; children = conv.comment;
        };

        const element = {
            dot: icon,
            // color: 'red',
            children: children
        }
        return element
    });

    const tab_items: TabsProps['items'] = [
        {
            key: '1',
            label: 'Conversation',
            children:
                <Space style={{ width: '100%' }}>
                    <Timeline items={conv_items} />
                </Space>
        },
        {
            key: '2',
            label: 'Files Changed',
            children: <Space style={{ width: '100%' }}>
                <List
                    // style={{ width: '100%' }}
                    header={<div>Change File List</div>}
                    bordered
                    dataSource={filedata}
                    loading={loadings[2]}
                    renderItem={(item) => (
                        <List.Item>
                            {item}
                        </List.Item>
                    )}
                />
            </Space>
        }
    ];

    return (
        <Card title={mrDetail.title + " #" + params.id}>
            {mrDetail && mrDetail.status === "open" &&
                <Button
                    type="primary"
                    loading={loadings[1]}
                    onClick={() => approve_mr(1, params.id)}
                >
                    Merge MR
                </Button>
            }
            <Tabs defaultActiveKey="1" items={tab_items} />
        </Card>
    )
}
