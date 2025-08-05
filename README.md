# SwapBytes libp2p App

## Installation and running the application

Install and open the application directory
```
    git clone https://eng-git.canterbury.ac.nz/rsc104/cosc473-assignment-2.git 
```

```
    cd cosc473-assignement2
```

First start up our rendezvous server so that we can discover peers on the network 
```
    cargo run --bin server
```

Then run the client(s), and optionally add the debug command to see system messages
(note that these can be a bit verbose so they aren't recommended for the average user)
```
    cargo run --bin client -- --debug
```

## Using the application

The server must simply be started to bootstrap the network (there is no hole-punching, but it is 
required for peer discovery). On the other hand, the client has a number of commands that will be 
required to use the app. These are each documented within the application, however, here are some examples 
of application use cases.

### Entering your name

When you open the app, you will be asked to enter your name. Simply type your name into the command
box, and then press Enter. At this stage, you may start to see other user's on the peers if they have
already entered their name.

#### Commands
```
    your username here
```

### Choosing a pubsub

After you enter your name, you will be prompted to join a pubsub. Say you want to join a pubsub with
the topic "UC Notes", either type in "UC Notes" if it already exists, or type '/new UC Notes' otherwise.
If you had to create a new pubsub, then you must wait for it to be added to the kademlia DHT, and then
you will be able to join it by typing "UC Notes" (this may take some time, as we only poll the DHT for
a list of topics every 20 seconds).

#### Commands
```
    a topic you want to join here
```
```
    /new UC Notes
```


### Finding a peer to trade files with

When you have joined a pubsub, you will be able to send messages to peers within that pubsub. For example, 
you might want to say "I'm looking for SENG401 week 3 notes, and I have all the COSC473 notes." If they
have the week 4 SENG401 notes, and need the COSC473 week 5 notes, you can send them a DM to exchange files! 
If the pubsub is not for you, then you can type "/exit" and join a different pubsub.

#### Commands
```
    a topic you want to join here
```
```
    /new UC Notes
```

```
    /exit
```

### DMing a peer

To DM a peer, you must be in a pubsub and have discovered this peer (you must be able to see their name in the 
peers box!) and then type "/dm peer", where peer is either the peer's full username, or at least the first 10 
characters of their peer id (both of which can be seen in the peers tab, however, usernames are not unique).

After joining the DM, the peer will be notified that you have attempted to DM them! Messages you send will also
be previewed by the other peer, and they will be able to see how many messages are pending from you.

Once you are done DMing your peer, you can type /exit to go back to the pubsub selection screen.

#### Commands
```
    /dm Bob
```

```
    /dm at_least_first_eight_characters_of_bob's_peer_id
```

```
    /exit
```

### Sharing files

Once you and Bob have agreed on files to share, and you are still in the DM with Bob, you must both use the commands 
/req and /res to request the file you want and respond with the file that the other user is requesting. This 
doesn't have to be done in any particular order.

The files that you have requested, and are responded with, will be added to your automatically generated peer id file. 

#### Commands
```
    /req seng_401_week_4.pdf
```

```
    /res cosc_473_week_5.pdf
```

### Reputation

Now, unfortunately Bob broke his promise and did not send back the SENG401 week 4 notes. Wanting to ensure that justice
is served, you quickly type in "-1" in your DM. Now Bob has a reputation of "-1" which is displayed to all other peers,
and this is stored for the next week.

But then you see an oddly named file "seng_401_week_4.pdf" pop up in your peer id directory. Oh no... Bob did hold up his
promise but the file took a couple of minutes to be received. Thankfully, you can also type "+1" to update your most recent
rating of a peer, and give Bob his well deserved "+1" reputation.

#### Commands
```
    +1
```

```
    -1
```

### Exiting the application

Out of embarrassment with your interaction with Bob, you decide it's time to close the application for today. Simply
press ctrl and q at the same time and the application will gracefully close.

#### Commands
```
    ctrl q
```