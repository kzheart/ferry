/** 串行化落盘:后一次写等前一次结束,前一次失败不阻塞后续排队。 */
export class WriteQueue {
  private tail: Promise<unknown> = Promise.resolve();

  run<T>(action: () => Promise<T>): Promise<T> {
    const next = this.tail.catch(() => undefined).then(action);
    this.tail = next;
    return next;
  }

  /** 等已排队的写全部结束;最后一次写失败会把错误抛出来。 */
  async settled() {
    await this.tail;
  }
}
