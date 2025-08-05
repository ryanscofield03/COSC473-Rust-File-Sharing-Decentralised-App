use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Paragraph};
use textwrap::fill;

/// generic trait for rendering a screen
pub(crate) trait ScreenRenderer {
    fn render(&self, frame: &mut Frame, area: Rect);
}

/// holds enter name screen information, and implements its render method
pub(crate) struct EnterNameScreen;
impl ScreenRenderer for EnterNameScreen {

    /// renders a frame of the enter name screen
    fn render(&self, frame: &mut Frame, area: Rect) {
        // break screen into blocks
        let blocks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(4)
            ])
            .split(area);

        // style and render the title block
        let title_block = Block::default()
            .borders(Borders::ALL)
            .title("Enter Your Name");
        frame.render_widget(title_block, blocks[0]);

        // style and render the instructions block
        let instructions = Paragraph::new(
            "- Type your username and press enter\n\
            - 'ctrl + q' to exit the application\n"
        )
            .block(Block::default().borders(Borders::ALL).title("Instructions"));
        frame.render_widget(instructions, blocks[1]);
    }
}

/// holds information for the select chatroom screen
pub(crate) struct SelectChatroomScreen {
    pub(crate) topics: Vec<String>,
}
impl ScreenRenderer for SelectChatroomScreen {

    /// renders a frame of the select chatroom screen
    fn render(&self, frame: &mut Frame, area: Rect) {
        // breaks screen into blocks
        let blocks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(5),
                Constraint::Min(10)
            ])
            .split(area);

        // style and render the title block
        let title_block = Block::default()
            .borders(Borders::ALL)
            .title("Select Chatroom");
        frame.render_widget(title_block, blocks[0]);

        // style and render the instructions block
        let instructions_block = Paragraph::new(
            "- Type an existing topic and press enter\n\
            - '/new topic_name' to create a new topic and press enter \
                (be patient while waiting for this to be stored in the DHT!)\n\
            - 'ctrl + q' to exit the application\n"
        )
            .block(Block::default().borders(Borders::ALL).title("Instructions"));
        frame.render_widget(instructions_block, blocks[1]);

        // style and render the selectable topics block
        let topics_block = Paragraph::new(self.topics.join("\n"))
            .block(Block::default().borders(Borders::ALL).title("Topics"));
        frame.render_widget(topics_block, blocks[2]);
    }
}

/// stores information for the chatroom screen
pub(crate) struct ChatroomScreen {
    pub(crate) topic: String,
    pub(crate) messages: Vec<String>,
}
impl ScreenRenderer for ChatroomScreen {

    /// renders a frame of the chatroom screen
    fn render(&self, frame: &mut Frame, area: Rect) {
        // breaks the screen into blocks
        let blocks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(5),
                Constraint::Min(6)
            ])
            .split(area);

        // styles and renders the title block
        let title_block = Block::default()
            .borders(Borders::ALL)
            .title(format!("Chatroom: {}", self.topic));
        frame.render_widget(title_block, blocks[0]);

        // styles and renders the instructions block
        let instructions_block = Paragraph::new(
            "- '/dm' followed by a username or at least the first eight characters of a peer id to DM\n\
            - '/exit' to choose a different topic\n\
            - 'ctrl + q' to exit the application\n"
        )
            .block(Block::default().borders(Borders::ALL).title("Instructions"));
        frame.render_widget(instructions_block, blocks[1]);

        // handles line wrapping and scroll
        let lines = self.messages.join("\n");
        let wrapped_lines = fill(&lines, (blocks[2].width - 2) as usize);
        let scroll = calculate_vertical_scroll(&wrapped_lines, blocks[2].height - 2);

        // styles and renders the instructions block
        let messages_block = Paragraph::new(self.messages.join("\n"))
            .block(Block::default().borders(Borders::ALL).title("Messages"))
            .scroll((scroll, 0)
            );
        frame.render_widget(messages_block, blocks[2]);
    }
}

/// stores information for the direct message screen
pub(crate) struct DirectMessageScreen {
    pub(crate) messages: Vec<String>,
    pub(crate) other_user_name: String,
}
impl ScreenRenderer for DirectMessageScreen {

    /// renders a frame of the direct message screen
    fn render(&self, frame: &mut Frame, area: Rect) {
        // breaks the screen into blocks
        let blocks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(8),
                Constraint::Min(10)
            ])
            .split(area);

        // styles and renders the title block
        let title_block = Block::default()
            .borders(Borders::ALL)
            .title(format!("Direct Message: {}", self.other_user_name));
        frame.render_widget(title_block, blocks[0]);

        // styles and renders the instructions block
        let instructions_block = Paragraph::new(
            "- '/req file_name' to request a file from your peer\n\
            - '/res file_name' to accept a file request from your peer\n\
            - '+1' if your peer followed up on their promise!\n\
            - '-1' if your peer did not follow up on their promise\n\
            - '/exit' to go back to the chatroom selection screen\n\
            - 'ctrl + q' to exit the application\n"
        )
            .block(Block::default().borders(Borders::ALL).title("Instructions"));
        frame.render_widget(instructions_block, blocks[1]);

        // styles and renders the messages block
        let messages_block = Paragraph::new(self.messages.join("\n"))
            .block(Block::default().borders(Borders::ALL).title("Messages"));
        frame.render_widget(messages_block, blocks[2]);
    }
}

/// renders system messages if we are in debug mode
pub(crate) fn render_system_messages(system_messages: Vec<String>, frame: &mut Frame, area: Rect) {
    // handles line wrapping and scroll
    let lines = system_messages.join("\n");
    let wrapped_lines = fill(&lines, (area.width - 2) as usize);
    let scroll = calculate_vertical_scroll(&wrapped_lines, area.height - 2);

    // styles and renders the system messages block
    let system_messages_box = Paragraph::new(wrapped_lines)
        .block(Block::default().title("System Messages").borders(Borders::ALL))
        .style(Style::default().fg(Color::White))
        .scroll((scroll, 0));
    frame.render_widget(system_messages_box, area);
}

/// handles rendering the users found block
pub(crate) fn render_found_users(found_users: Vec<String>, frame: &mut Frame, area: Rect) {
    // handles line wrapping and scroll
    let lines = found_users.join("\n");
    let wrapped_lines = fill(&lines, (area.width - 2) as usize);
    let scroll = calculate_vertical_scroll(&wrapped_lines, area.height - 2);

    // styles and renders the system messages block
    let users_found_block = Paragraph::new(wrapped_lines)
        .block(Block::default().title("Peers").borders(Borders::ALL))
        .style(Style::default().fg(Color::White))
        .scroll((scroll, 0));
    frame.render_widget(users_found_block, area);
}

/// handles rendering the users found block
pub(crate) fn render_pending_dms(pending_dms: Vec<String>, frame: &mut Frame, area: Rect) {
    // handles line wrapping and scroll
    let lines = pending_dms.join("\n");
    let wrapped_lines = fill(&lines, (area.width - 2) as usize);
    let scroll = calculate_vertical_scroll(&wrapped_lines, area.height - 2);

    // styles and renders the system messages block
    let pending_dms_block = Paragraph::new(wrapped_lines)
        .block(Block::default().title("DMs").borders(Borders::ALL))
        .style(Style::default().fg(Color::White))
        .scroll((scroll, 0));
    frame.render_widget(pending_dms_block, area);
}

/// handles rendering the input box
pub(crate) fn render_input_box(current_input: String, frame: &mut Frame, area: Rect) {
    let input_box = Paragraph::new(current_input)
        .block(Block::default().title("Input").borders(Borders::ALL))
        .style(Style::default().fg(Color::Yellow));
    frame.render_widget(input_box, area);
}

/// calculates the vertical scroll given the wrapped lines of text and the height of the block
pub(crate) fn calculate_vertical_scroll(
    lines: &str,
    height: u16,
) -> u16 {
    lines.lines().count().saturating_sub((height) as usize).max(0) as u16
}