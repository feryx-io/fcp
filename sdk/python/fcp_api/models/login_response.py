from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.auth_user_response import AuthUserResponse





T = TypeVar("T", bound="LoginResponse")



@_attrs_define
class LoginResponse:
    """ 
        Attributes:
            user (AuthUserResponse):
            token (None | str | Unset): The access token: `Authorization: Bearer <token>`. Absent while a
                second factor is wanted (`mfa_required`) or the password must be
                changed (`password_expired`).
            expires_in (int | None | Unset): Seconds the access token lasts; refresh before then.
            password_expired (bool | None | Unset):
            mfa_required (None | str | Unset): The password was right; now a second factor
                (`docs/IDENTITY_AND_DEVICES.md` phase 2b): `totp` — a code to
                `/auth/login/totp` — or `totp_enrollment` — the tenant requires one
                the user has not set up: `/auth/totp/enroll` then `/auth/totp/confirm`.
            mfa_token (None | str | Unset): The ticket those routes take, valid five minutes.
            recovery_codes (list[str] | None | Unset): Recovery codes, shown once, when a second factor was just set up.
     """

    user: AuthUserResponse
    token: None | str | Unset = UNSET
    expires_in: int | None | Unset = UNSET
    password_expired: bool | None | Unset = UNSET
    mfa_required: None | str | Unset = UNSET
    mfa_token: None | str | Unset = UNSET
    recovery_codes: list[str] | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.auth_user_response import AuthUserResponse # noqa: PLC0415
        user = self.user.to_dict()

        token: None | str | Unset
        if isinstance(self.token, Unset):
            token = UNSET
        else:
            token = self.token

        expires_in: int | None | Unset
        if isinstance(self.expires_in, Unset):
            expires_in = UNSET
        else:
            expires_in = self.expires_in

        password_expired: bool | None | Unset
        if isinstance(self.password_expired, Unset):
            password_expired = UNSET
        else:
            password_expired = self.password_expired

        mfa_required: None | str | Unset
        if isinstance(self.mfa_required, Unset):
            mfa_required = UNSET
        else:
            mfa_required = self.mfa_required

        mfa_token: None | str | Unset
        if isinstance(self.mfa_token, Unset):
            mfa_token = UNSET
        else:
            mfa_token = self.mfa_token

        recovery_codes: list[str] | None | Unset
        if isinstance(self.recovery_codes, Unset):
            recovery_codes = UNSET
        elif isinstance(self.recovery_codes, list):
            recovery_codes = self.recovery_codes


        else:
            recovery_codes = self.recovery_codes


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "user": user,
        })
        if token is not UNSET:
            field_dict["token"] = token
        if expires_in is not UNSET:
            field_dict["expires_in"] = expires_in
        if password_expired is not UNSET:
            field_dict["password_expired"] = password_expired
        if mfa_required is not UNSET:
            field_dict["mfa_required"] = mfa_required
        if mfa_token is not UNSET:
            field_dict["mfa_token"] = mfa_token
        if recovery_codes is not UNSET:
            field_dict["recovery_codes"] = recovery_codes

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.auth_user_response import AuthUserResponse # noqa: PLC0415
        d = dict(src_dict)
        user = AuthUserResponse.from_dict(d.pop("user"))




        def _parse_token(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        token = _parse_token(d.pop("token", UNSET))


        def _parse_expires_in(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        expires_in = _parse_expires_in(d.pop("expires_in", UNSET))


        def _parse_password_expired(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        password_expired = _parse_password_expired(d.pop("password_expired", UNSET))


        def _parse_mfa_required(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        mfa_required = _parse_mfa_required(d.pop("mfa_required", UNSET))


        def _parse_mfa_token(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        mfa_token = _parse_mfa_token(d.pop("mfa_token", UNSET))


        def _parse_recovery_codes(data: object) -> list[str] | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, list):
                    raise TypeError()
                recovery_codes_type_0 = cast(list[str], data)

                return recovery_codes_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(list[str] | None | Unset, data)

        recovery_codes = _parse_recovery_codes(d.pop("recovery_codes", UNSET))


        login_response = cls(
            user=user,
            token=token,
            expires_in=expires_in,
            password_expired=password_expired,
            mfa_required=mfa_required,
            mfa_token=mfa_token,
            recovery_codes=recovery_codes,
        )


        login_response.additional_properties = d
        return login_response

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
