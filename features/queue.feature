Feature: Queue
  A person queues issues on their own companion and starts them in their own time.

  @queue.queue-on-own-lane
  Scenario: Dropping an issue on your own lane queues it there
    Given a person's companion is paired
    When they drag a pinned issue onto their own lane
    Then the issue is queued on that companion, marked as queued by them

  @queue.refused-start-stays
  Scenario: A queued issue the companion refuses to start stays and says why
    Given an issue queued on a person's companion
    When they start it and the companion refuses
    Then the entry stays in the queue with the companion's reason

  @queue.start-leaves-queue
  Scenario: A queued issue leaves the queue when its session starts
    Given an issue queued on a person's companion
    When they start it and the companion reports it started
    Then the entry leaves the queue

  @queue.reorder
  Scenario: A person reorders their own queue
    Given two issues queued on a person's companion
    When they move the second above the first
    Then the queue shows them in the new order for everyone
