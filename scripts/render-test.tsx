import React from "react";
import { renderToString } from "react-dom/server";
import ObjectTable from "/Users/jeff/workspace/github/sssc/src/components/ObjectTable";
import * as fs from "fs";

const list = JSON.parse(fs.readFileSync("/tmp/sssc-real-list.json", "utf8"));
const buckets = JSON.parse(fs.readFileSync("/tmp/sssc-real-buckets.json", "utf8"));

// 场景1：桶列表
try {
  renderToString(<ObjectTable loading={false} list={null} buckets={buckets} bucket={null} prefix="" profileId="p1"
    onEnterPrefix={() => {}} onSelectBucket={() => {}} onUp={() => {}} onSelectObject={() => {}}
    onError={() => {}} onRefresh={() => {}} onDownload={() => {}} onDeleteBucket={() => {}} />);
  console.log("场景1（桶列表）渲染 OK");
} catch (e) { console.log("场景1 崩溃:", e); }

// 场景2：对象列表
try {
  renderToString(<ObjectTable loading={false} list={list} buckets={buckets} bucket="hdd-demo" prefix="" profileId="p1"
    onEnterPrefix={() => {}} onSelectBucket={() => {}} onUp={() => {}} onSelectObject={() => {}}
    onError={() => {}} onRefresh={() => {}} onDownload={() => {}} onDeleteBucket={() => {}} />);
  console.log("场景2（对象列表）渲染 OK");
} catch (e) { console.log("场景2 崩溃:", e); }
