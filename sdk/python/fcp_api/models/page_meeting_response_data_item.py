from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.video_codec import VideoCodec
from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="PageMeetingResponseDataItem")



@_attrs_define
class PageMeetingResponseDataItem:
    """ Meeting response.

        Attributes:
            id (str):
            tenant_id (str):
            room_number (str):
            name (str):
            require_guest_pin (bool):
            max_participants (int):
            wait_for_moderator (bool):
            enabled (bool):
            auto_record (bool):
            default_locked (bool):
            permanent (bool):
            play_entry_tone (bool):
            play_exit_tone (bool):
            video_enabled (bool):
            video_layout (str):
            video_max_participants (int):
            video_max_resolution (str):
            video_fps (int):
            video_exclude_self (bool):
            video_codecs (list[VideoCodec]):
            video_passthrough (str): When a participant whose view is a single source they can decode
                is sent that source's own stream: `auto`, `on` or `off` (§5.6).
            video_content_share (str): Who may share a screen: `off`, `hosts` or `anyone` (§15.6).
            video_content_layout (str): How the composite shows a shared screen: `presentation` or
                `content_only`.
            video_mode (str): Who is sent the composite: `mix` (everyone) or `auto` (a client
                that asks for each person's own stream gets them instead).
            kind (str): `meeting` or `webinar` (`docs/WEBINARS.md` §2.1).
            panelists (list[str]): A webinar's panelists, users by id.
            created_at (datetime.datetime):
            updated_at (datetime.datetime):
            guest_pin (None | str | Unset):
            host_pin (None | str | Unset):
            owner_id (None | str | Unset):
            pin_prompt_sound (None | str | Unset):
            invalid_pin_sound (None | str | Unset):
            moh_sound (None | str | Unset):
            pin_prompt_tts (None | str | Unset):
            invalid_pin_tts (None | str | Unset):
            welcome_tts (None | str | Unset):
            join_sound (None | str | Unset):
            exit_sound (None | str | Unset):
            panelist_pin (None | str | Unset): A webinar's PIN for panelists.
     """

    id: str
    tenant_id: str
    room_number: str
    name: str
    require_guest_pin: bool
    max_participants: int
    wait_for_moderator: bool
    enabled: bool
    auto_record: bool
    default_locked: bool
    permanent: bool
    play_entry_tone: bool
    play_exit_tone: bool
    video_enabled: bool
    video_layout: str
    video_max_participants: int
    video_max_resolution: str
    video_fps: int
    video_exclude_self: bool
    video_codecs: list[VideoCodec]
    video_passthrough: str
    video_content_share: str
    video_content_layout: str
    video_mode: str
    kind: str
    panelists: list[str]
    created_at: datetime.datetime
    updated_at: datetime.datetime
    guest_pin: None | str | Unset = UNSET
    host_pin: None | str | Unset = UNSET
    owner_id: None | str | Unset = UNSET
    pin_prompt_sound: None | str | Unset = UNSET
    invalid_pin_sound: None | str | Unset = UNSET
    moh_sound: None | str | Unset = UNSET
    pin_prompt_tts: None | str | Unset = UNSET
    invalid_pin_tts: None | str | Unset = UNSET
    welcome_tts: None | str | Unset = UNSET
    join_sound: None | str | Unset = UNSET
    exit_sound: None | str | Unset = UNSET
    panelist_pin: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        tenant_id = self.tenant_id

        room_number = self.room_number

        name = self.name

        require_guest_pin = self.require_guest_pin

        max_participants = self.max_participants

        wait_for_moderator = self.wait_for_moderator

        enabled = self.enabled

        auto_record = self.auto_record

        default_locked = self.default_locked

        permanent = self.permanent

        play_entry_tone = self.play_entry_tone

        play_exit_tone = self.play_exit_tone

        video_enabled = self.video_enabled

        video_layout = self.video_layout

        video_max_participants = self.video_max_participants

        video_max_resolution = self.video_max_resolution

        video_fps = self.video_fps

        video_exclude_self = self.video_exclude_self

        video_codecs = []
        for video_codecs_item_data in self.video_codecs:
            video_codecs_item = video_codecs_item_data.value
            video_codecs.append(video_codecs_item)



        video_passthrough = self.video_passthrough

        video_content_share = self.video_content_share

        video_content_layout = self.video_content_layout

        video_mode = self.video_mode

        kind = self.kind

        panelists = self.panelists



        created_at = self.created_at.isoformat()

        updated_at = self.updated_at.isoformat()

        guest_pin: None | str | Unset
        if isinstance(self.guest_pin, Unset):
            guest_pin = UNSET
        else:
            guest_pin = self.guest_pin

        host_pin: None | str | Unset
        if isinstance(self.host_pin, Unset):
            host_pin = UNSET
        else:
            host_pin = self.host_pin

        owner_id: None | str | Unset
        if isinstance(self.owner_id, Unset):
            owner_id = UNSET
        else:
            owner_id = self.owner_id

        pin_prompt_sound: None | str | Unset
        if isinstance(self.pin_prompt_sound, Unset):
            pin_prompt_sound = UNSET
        else:
            pin_prompt_sound = self.pin_prompt_sound

        invalid_pin_sound: None | str | Unset
        if isinstance(self.invalid_pin_sound, Unset):
            invalid_pin_sound = UNSET
        else:
            invalid_pin_sound = self.invalid_pin_sound

        moh_sound: None | str | Unset
        if isinstance(self.moh_sound, Unset):
            moh_sound = UNSET
        else:
            moh_sound = self.moh_sound

        pin_prompt_tts: None | str | Unset
        if isinstance(self.pin_prompt_tts, Unset):
            pin_prompt_tts = UNSET
        else:
            pin_prompt_tts = self.pin_prompt_tts

        invalid_pin_tts: None | str | Unset
        if isinstance(self.invalid_pin_tts, Unset):
            invalid_pin_tts = UNSET
        else:
            invalid_pin_tts = self.invalid_pin_tts

        welcome_tts: None | str | Unset
        if isinstance(self.welcome_tts, Unset):
            welcome_tts = UNSET
        else:
            welcome_tts = self.welcome_tts

        join_sound: None | str | Unset
        if isinstance(self.join_sound, Unset):
            join_sound = UNSET
        else:
            join_sound = self.join_sound

        exit_sound: None | str | Unset
        if isinstance(self.exit_sound, Unset):
            exit_sound = UNSET
        else:
            exit_sound = self.exit_sound

        panelist_pin: None | str | Unset
        if isinstance(self.panelist_pin, Unset):
            panelist_pin = UNSET
        else:
            panelist_pin = self.panelist_pin


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "tenant_id": tenant_id,
            "room_number": room_number,
            "name": name,
            "require_guest_pin": require_guest_pin,
            "max_participants": max_participants,
            "wait_for_moderator": wait_for_moderator,
            "enabled": enabled,
            "auto_record": auto_record,
            "default_locked": default_locked,
            "permanent": permanent,
            "play_entry_tone": play_entry_tone,
            "play_exit_tone": play_exit_tone,
            "video_enabled": video_enabled,
            "video_layout": video_layout,
            "video_max_participants": video_max_participants,
            "video_max_resolution": video_max_resolution,
            "video_fps": video_fps,
            "video_exclude_self": video_exclude_self,
            "video_codecs": video_codecs,
            "video_passthrough": video_passthrough,
            "video_content_share": video_content_share,
            "video_content_layout": video_content_layout,
            "video_mode": video_mode,
            "kind": kind,
            "panelists": panelists,
            "created_at": created_at,
            "updated_at": updated_at,
        })
        if guest_pin is not UNSET:
            field_dict["guest_pin"] = guest_pin
        if host_pin is not UNSET:
            field_dict["host_pin"] = host_pin
        if owner_id is not UNSET:
            field_dict["owner_id"] = owner_id
        if pin_prompt_sound is not UNSET:
            field_dict["pin_prompt_sound"] = pin_prompt_sound
        if invalid_pin_sound is not UNSET:
            field_dict["invalid_pin_sound"] = invalid_pin_sound
        if moh_sound is not UNSET:
            field_dict["moh_sound"] = moh_sound
        if pin_prompt_tts is not UNSET:
            field_dict["pin_prompt_tts"] = pin_prompt_tts
        if invalid_pin_tts is not UNSET:
            field_dict["invalid_pin_tts"] = invalid_pin_tts
        if welcome_tts is not UNSET:
            field_dict["welcome_tts"] = welcome_tts
        if join_sound is not UNSET:
            field_dict["join_sound"] = join_sound
        if exit_sound is not UNSET:
            field_dict["exit_sound"] = exit_sound
        if panelist_pin is not UNSET:
            field_dict["panelist_pin"] = panelist_pin

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        tenant_id = d.pop("tenant_id")

        room_number = d.pop("room_number")

        name = d.pop("name")

        require_guest_pin = d.pop("require_guest_pin")

        max_participants = d.pop("max_participants")

        wait_for_moderator = d.pop("wait_for_moderator")

        enabled = d.pop("enabled")

        auto_record = d.pop("auto_record")

        default_locked = d.pop("default_locked")

        permanent = d.pop("permanent")

        play_entry_tone = d.pop("play_entry_tone")

        play_exit_tone = d.pop("play_exit_tone")

        video_enabled = d.pop("video_enabled")

        video_layout = d.pop("video_layout")

        video_max_participants = d.pop("video_max_participants")

        video_max_resolution = d.pop("video_max_resolution")

        video_fps = d.pop("video_fps")

        video_exclude_self = d.pop("video_exclude_self")

        video_codecs = []
        _video_codecs = d.pop("video_codecs")
        for video_codecs_item_data in (_video_codecs):
            video_codecs_item = VideoCodec(video_codecs_item_data)



            video_codecs.append(video_codecs_item)


        video_passthrough = d.pop("video_passthrough")

        video_content_share = d.pop("video_content_share")

        video_content_layout = d.pop("video_content_layout")

        video_mode = d.pop("video_mode")

        kind = d.pop("kind")

        panelists = cast(list[str], d.pop("panelists"))


        created_at = datetime.datetime.fromisoformat(d.pop("created_at"))




        updated_at = datetime.datetime.fromisoformat(d.pop("updated_at"))




        def _parse_guest_pin(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        guest_pin = _parse_guest_pin(d.pop("guest_pin", UNSET))


        def _parse_host_pin(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        host_pin = _parse_host_pin(d.pop("host_pin", UNSET))


        def _parse_owner_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        owner_id = _parse_owner_id(d.pop("owner_id", UNSET))


        def _parse_pin_prompt_sound(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        pin_prompt_sound = _parse_pin_prompt_sound(d.pop("pin_prompt_sound", UNSET))


        def _parse_invalid_pin_sound(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        invalid_pin_sound = _parse_invalid_pin_sound(d.pop("invalid_pin_sound", UNSET))


        def _parse_moh_sound(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        moh_sound = _parse_moh_sound(d.pop("moh_sound", UNSET))


        def _parse_pin_prompt_tts(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        pin_prompt_tts = _parse_pin_prompt_tts(d.pop("pin_prompt_tts", UNSET))


        def _parse_invalid_pin_tts(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        invalid_pin_tts = _parse_invalid_pin_tts(d.pop("invalid_pin_tts", UNSET))


        def _parse_welcome_tts(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        welcome_tts = _parse_welcome_tts(d.pop("welcome_tts", UNSET))


        def _parse_join_sound(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        join_sound = _parse_join_sound(d.pop("join_sound", UNSET))


        def _parse_exit_sound(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        exit_sound = _parse_exit_sound(d.pop("exit_sound", UNSET))


        def _parse_panelist_pin(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        panelist_pin = _parse_panelist_pin(d.pop("panelist_pin", UNSET))


        page_meeting_response_data_item = cls(
            id=id,
            tenant_id=tenant_id,
            room_number=room_number,
            name=name,
            require_guest_pin=require_guest_pin,
            max_participants=max_participants,
            wait_for_moderator=wait_for_moderator,
            enabled=enabled,
            auto_record=auto_record,
            default_locked=default_locked,
            permanent=permanent,
            play_entry_tone=play_entry_tone,
            play_exit_tone=play_exit_tone,
            video_enabled=video_enabled,
            video_layout=video_layout,
            video_max_participants=video_max_participants,
            video_max_resolution=video_max_resolution,
            video_fps=video_fps,
            video_exclude_self=video_exclude_self,
            video_codecs=video_codecs,
            video_passthrough=video_passthrough,
            video_content_share=video_content_share,
            video_content_layout=video_content_layout,
            video_mode=video_mode,
            kind=kind,
            panelists=panelists,
            created_at=created_at,
            updated_at=updated_at,
            guest_pin=guest_pin,
            host_pin=host_pin,
            owner_id=owner_id,
            pin_prompt_sound=pin_prompt_sound,
            invalid_pin_sound=invalid_pin_sound,
            moh_sound=moh_sound,
            pin_prompt_tts=pin_prompt_tts,
            invalid_pin_tts=invalid_pin_tts,
            welcome_tts=welcome_tts,
            join_sound=join_sound,
            exit_sound=exit_sound,
            panelist_pin=panelist_pin,
        )


        page_meeting_response_data_item.additional_properties = d
        return page_meeting_response_data_item

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
