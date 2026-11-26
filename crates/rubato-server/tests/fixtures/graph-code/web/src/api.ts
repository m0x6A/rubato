import { format } from "./format";
import type { Me } from "../protocol.js";
import React from "react";
import { z } from "@scope/pkg/deep";
import { readFile } from "node:fs";

/** Requests a path. */
export async function request(path: string): Promise<Me> {
  return JSON.parse(format(path));
}

export interface ApiError {
  status: number;
}

export class Client {
  get(path: string) {
    return request(path);
  }
}

export const later = (ms: number) => new Promise((r) => setTimeout(r, ms));
