from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="StreamLinkResponse")



@_attrs_define
class StreamLinkResponse:
    """ A stream link.

        Attributes:
            token (str): The signed token, the credential.
            playlist_path (str): The playlist on this admin, for a player.
            watch_path (str): The watch page on this admin (W8): the stream in a browser, no
                client needed.
            expires_at (datetime.datetime):
            playlist_url (None | str | Unset): The same with this admin's public address, when it knows it
                (`[api.sso] public_url`).
            watch_url (None | str | Unset): The same with this admin's public address, when it knows it.
     """

    token: str
    playlist_path: str
    watch_path: str
    expires_at: datetime.datetime
    playlist_url: None | str | Unset = UNSET
    watch_url: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        token = self.token

        playlist_path = self.playlist_path

        watch_path = self.watch_path

        expires_at = self.expires_at.isoformat()

        playlist_url: None | str | Unset
        if isinstance(self.playlist_url, Unset):
            playlist_url = UNSET
        else:
            playlist_url = self.playlist_url

        watch_url: None | str | Unset
        if isinstance(self.watch_url, Unset):
            watch_url = UNSET
        else:
            watch_url = self.watch_url


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "token": token,
            "playlist_path": playlist_path,
            "watch_path": watch_path,
            "expires_at": expires_at,
        })
        if playlist_url is not UNSET:
            field_dict["playlist_url"] = playlist_url
        if watch_url is not UNSET:
            field_dict["watch_url"] = watch_url

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        token = d.pop("token")

        playlist_path = d.pop("playlist_path")

        watch_path = d.pop("watch_path")

        expires_at = datetime.datetime.fromisoformat(d.pop("expires_at"))




        def _parse_playlist_url(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        playlist_url = _parse_playlist_url(d.pop("playlist_url", UNSET))


        def _parse_watch_url(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        watch_url = _parse_watch_url(d.pop("watch_url", UNSET))


        stream_link_response = cls(
            token=token,
            playlist_path=playlist_path,
            watch_path=watch_path,
            expires_at=expires_at,
            playlist_url=playlist_url,
            watch_url=watch_url,
        )


        stream_link_response.additional_properties = d
        return stream_link_response

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
