from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from typing import cast

if TYPE_CHECKING:
  from ..models.auth_user_response import AuthUserResponse





T = TypeVar("T", bound="AppTokenResponse")



@_attrs_define
class AppTokenResponse:
    """ An app session's tokens.

        Attributes:
            access_token (str):
            token_type (str): `Bearer`.
            expires_in (int): Seconds the access token lasts.
            refresh_token (str): Opaque; rotated on every refresh, so keep the newest.
            refresh_expires_in (int): Seconds the session lasts unless refreshed.
            session_id (str): The session, as `/me/sessions` and the audit log name it.
            user (AuthUserResponse):
     """

    access_token: str
    token_type: str
    expires_in: int
    refresh_token: str
    refresh_expires_in: int
    session_id: str
    user: AuthUserResponse
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.auth_user_response import AuthUserResponse # noqa: PLC0415
        access_token = self.access_token

        token_type = self.token_type

        expires_in = self.expires_in

        refresh_token = self.refresh_token

        refresh_expires_in = self.refresh_expires_in

        session_id = self.session_id

        user = self.user.to_dict()


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "access_token": access_token,
            "token_type": token_type,
            "expires_in": expires_in,
            "refresh_token": refresh_token,
            "refresh_expires_in": refresh_expires_in,
            "session_id": session_id,
            "user": user,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.auth_user_response import AuthUserResponse # noqa: PLC0415
        d = dict(src_dict)
        access_token = d.pop("access_token")

        token_type = d.pop("token_type")

        expires_in = d.pop("expires_in")

        refresh_token = d.pop("refresh_token")

        refresh_expires_in = d.pop("refresh_expires_in")

        session_id = d.pop("session_id")

        user = AuthUserResponse.from_dict(d.pop("user"))




        app_token_response = cls(
            access_token=access_token,
            token_type=token_type,
            expires_in=expires_in,
            refresh_token=refresh_token,
            refresh_expires_in=refresh_expires_in,
            session_id=session_id,
            user=user,
        )


        app_token_response.additional_properties = d
        return app_token_response

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
