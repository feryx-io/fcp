from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.app_device_response import AppDeviceResponse
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
            device (AppDeviceResponse | None | Unset):
     """

    access_token: str
    token_type: str
    expires_in: int
    refresh_token: str
    refresh_expires_in: int
    session_id: str
    user: AuthUserResponse
    device: AppDeviceResponse | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.app_device_response import AppDeviceResponse # noqa: PLC0415
        from ..models.auth_user_response import AuthUserResponse # noqa: PLC0415
        access_token = self.access_token

        token_type = self.token_type

        expires_in = self.expires_in

        refresh_token = self.refresh_token

        refresh_expires_in = self.refresh_expires_in

        session_id = self.session_id

        user = self.user.to_dict()

        device: dict[str, Any] | None | Unset
        if isinstance(self.device, Unset):
            device = UNSET
        elif isinstance(self.device, AppDeviceResponse):
            device = self.device.to_dict()
        else:
            device = self.device


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
        if device is not UNSET:
            field_dict["device"] = device

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.app_device_response import AppDeviceResponse # noqa: PLC0415
        from ..models.auth_user_response import AuthUserResponse # noqa: PLC0415
        d = dict(src_dict)
        access_token = d.pop("access_token")

        token_type = d.pop("token_type")

        expires_in = d.pop("expires_in")

        refresh_token = d.pop("refresh_token")

        refresh_expires_in = d.pop("refresh_expires_in")

        session_id = d.pop("session_id")

        user = AuthUserResponse.from_dict(d.pop("user"))




        def _parse_device(data: object) -> AppDeviceResponse | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                device_type_0 = AppDeviceResponse.from_dict(data)



                return device_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(AppDeviceResponse | None | Unset, data)

        device = _parse_device(d.pop("device", UNSET))


        app_token_response = cls(
            access_token=access_token,
            token_type=token_type,
            expires_in=expires_in,
            refresh_token=refresh_token,
            refresh_expires_in=refresh_expires_in,
            session_id=session_id,
            user=user,
            device=device,
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
